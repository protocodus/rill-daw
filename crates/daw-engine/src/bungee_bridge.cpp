// Rill's MIT-licensed, exception-contained bridge to the unmodified Bungee DSP.
// Audio is prepared on a worker, never in the device callback. No source-sized
// planar copy is needed: the bounded grain buffer is reused throughout.
#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <vector>
#include "bungee/Bungee.h"

namespace {
// Output-time to source-position map shared with Rust's TimeMap: [source,
// output] knots, the clip rate outside them. Without knots the original
// constant-speed request sequence is used unchanged.
struct WarpMap {
    const double *knots;
    size_t count;
    double rate;
    double source(size_t i) const { return knots[2 * i]; }
    double output(size_t i) const { return knots[2 * i + 1]; }
    size_t segment(double t) const {
        size_t low = 0, high = count;
        while (low < high) {
            const size_t middle = (low + high) / 2;
            if (output(middle) <= t) low = middle + 1; else high = middle;
        }
        return low;
    }
    double position(double t) const {
        const size_t index = segment(t);
        if (index == 0) return source(0) + (t - output(0)) * rate;
        if (index == count) return source(count - 1) + (t - output(count - 1)) * rate;
        return source(index - 1) + (t - output(index - 1)) * speed(t);
    }
    double speed(double t) const {
        const size_t index = segment(t);
        if (index == 0 || index == count) return rate;
        return (source(index) - source(index - 1)) / (output(index) - output(index - 1));
    }
    bool valid() const {
        if (!count) return true;
        if (!knots || count > 4096) return false;
        for (size_t i = 0; i < count; ++i)
            if (!std::isfinite(source(i)) || !std::isfinite(output(i))) return false;
        for (size_t i = 1; i < count; ++i) {
            const double span = output(i) - output(i - 1);
            const double speed = (source(i) - source(i - 1)) / span;
            if (!(span > 0) || !(speed >= .25 * .999999) || !(speed <= 4 * 1.000001)) return false;
        }
        return true;
    }
};

// Advance one grain. With a warp map the synthesis hop is measured from the
// stretcher's own step, then the next grain is placed exactly on the map.
void advance(Bungee::Stretcher<Bungee::Basic> &dsp, Bungee::Request &request,
             const WarpMap &map, double &time) {
    const double before = request.position;
    dsp.next(request);
    if (!map.count) return;
    time += (request.position - before) / request.speed;
    request.position = map.position(time);
    request.speed = map.speed(time);
}
}

extern "C" int rill_bungee_process(
    const float *input, size_t input_frames, float *output, size_t output_frames,
    unsigned channels, unsigned sample_rate, double rate,
    const double *knots, size_t knot_count, double semitones,
    void *cancel_context, int (*cancelled)(void *)) noexcept {
    const WarpMap map{knots, knot_count, rate};
    if (!map.valid() || !input || !output || !cancelled || channels < 1 || channels > 2 ||
        sample_rate < 8000 || sample_rate > 384000 ||
        !std::isfinite(rate) || rate < .25 || rate > 4 ||
        !std::isfinite(semitones) || semitones < -24 || semitones > 24 ||
        input_frames > 134217728 || output_frames > 134217728)
        return 0;
    try {
        Bungee::Stretcher<Bungee::Basic> dsp({int(sample_rate), int(sample_rate)}, int(channels));
        const int capacity = dsp.maxInputFrameCount();
        if (capacity <= 0 || capacity > 1048576) return 0;
        std::vector<float> grain(size_t(capacity) * channels);
        Bungee::Request request{};
        request.position = 0;
        request.speed = rate;
        request.pitch = std::exp2(semitones / 12.0);
        if (map.count) request.speed = map.speed(0);
        dsp.preroll(request);
        // Preroll places the first grain before source zero at constant speed.
        double time = request.position / request.speed;
        if (map.count) request.position = map.position(time);
        size_t produced = 0;
        // Even one-sample hops must eventually finish. The additional allowance
        // covers latency at the highest accepted sample rate and pitch shift.
        const size_t maximum_grains = output_frames + 1048576;
        for (size_t iteration = 0; produced < output_frames && iteration < maximum_grains; ++iteration) {
            if (cancelled(cancel_context)) return -1;
            const double position = request.position;
            if (!std::isfinite(position) || position < -8.0 * capacity ||
                position > double(input_frames) + 8.0 * capacity) return 0;
            const Bungee::InputChunk chunk = dsp.specifyGrain(request);
            const auto count = int64_t(chunk.end) - chunk.begin;
            if (count < 0 || count > capacity) return 0;
            std::fill(grain.begin(), grain.end(), 0.f);
            const int begin = std::max(chunk.begin, 0);
            const int end = std::min<int64_t>(chunk.end, input_frames);
            for (int frame = begin; frame < end; ++frame) {
                for (unsigned channel = 0; channel < channels; ++channel)
                    grain[size_t(channel) * capacity + frame - chunk.begin] =
                        input[size_t(frame) * channels + channel];
            }
            const int mute_head = int(std::clamp<int64_t>(-int64_t(chunk.begin), 0, count));
            const int mute_tail = int(std::clamp<int64_t>(int64_t(chunk.end) - int64_t(input_frames), 0, count - mute_head));
            dsp.analyseGrain(grain.data(), capacity, mute_head, mute_tail);
            Bungee::OutputChunk rendered{};
            dsp.synthesiseGrain(rendered);
            advance(dsp, request, map, time);
            if (!(request.position > position)) return 0;
            if (!rendered.request[0] || !rendered.request[1] || rendered.frameCount < 0)
                return 0;
            const double start = rendered.request[0]->position;
            const double stop = rendered.request[1]->position;
            // Bungee's timestamped preroll includes audio before source frame
            // zero. Remove it once, then concatenate the non-overlapping grains.
            if (!std::isfinite(start) || !std::isfinite(stop) || stop <= start)
                continue;
            const double skip = std::max(0.0, std::round(-start * rendered.frameCount / (stop - start)));
            const int first = int(std::min<double>(skip, rendered.frameCount));
            const size_t frames = std::min<size_t>(rendered.frameCount - first, output_frames - produced);
            if (frames && (!rendered.data || rendered.channelStride < rendered.frameCount)) return 0;
            for (size_t frame = 0; frame < frames; ++frame) {
                for (unsigned channel = 0; channel < channels; ++channel)
                    output[(produced + frame) * channels + channel] =
                        rendered.data[size_t(channel) * rendered.channelStride + first + frame];
            }
            produced += frames;
        }
        return produced == output_frames ? 1 : 0;
    } catch (...) { return 0; }
}

// File preparation shares one continuous native instance, but requests only each
// bounded grain from Rust and returns each bounded block to its leased writer.
// No whole-file input/output allocation or temporary planar source is required.
extern "C" int rill_bungee_process_stream(
    size_t input_frames, size_t output_frames, unsigned channels,
    unsigned sample_rate, double rate, const double *knots, size_t knot_count,
    double semitones, void *context,
    int (*read)(void *, size_t, size_t, float *),
    int (*write)(void *, const float *, size_t),
    int (*cancelled)(void *)) noexcept {
    const WarpMap map{knots, knot_count, rate};
    if (!map.valid() || !context || !read || !write || !cancelled || channels < 1 || channels > 2 ||
        sample_rate < 8000 || sample_rate > 384000 ||
        !std::isfinite(rate) || rate < .25 || rate > 4 ||
        !std::isfinite(semitones) || semitones < -24 || semitones > 24 ||
        input_frames > size_t(std::numeric_limits<int64_t>::max() / 2) ||
        output_frames > size_t(std::numeric_limits<uint32_t>::max() / (sizeof(float) * channels)))
        return 0;
    try {
        Bungee::Stretcher<Bungee::Basic> dsp({int(sample_rate), int(sample_rate)}, int(channels));
        const int capacity = dsp.maxInputFrameCount();
        if (capacity <= 0 || capacity > 1048576) return 0;
        std::vector<float> grain(size_t(capacity) * channels);
        std::vector<float> input(size_t(capacity) * channels);
        std::vector<float> output;
        Bungee::Request request{};
        request.position = 0;
        request.speed = rate;
        request.pitch = std::exp2(semitones / 12.0);
        if (map.count) request.speed = map.speed(0);
        dsp.preroll(request);
        // Preroll places the first grain before source zero at constant speed.
        double time = request.position / request.speed;
        if (map.count) request.position = map.position(time);
        size_t produced = 0;
        const size_t maximum_grains = output_frames + 1048576;
        for (size_t iteration = 0; produced < output_frames && iteration < maximum_grains; ++iteration) {
            if (cancelled(context)) return -1;
            const double position = request.position;
            if (!std::isfinite(position) || position < -8.0 * capacity ||
                position > double(input_frames) + 8.0 * capacity) return 0;
            // Bungee exposes signed-int chunk offsets. Supplying an integral
            // local buffer origin keeps them bounded even for long source files,
            // while preserving round(position), including negative preroll ties.
            const int64_t origin = int64_t(std::trunc(position));
            const Bungee::InputChunk chunk = dsp.specifyGrain(request, double(origin));
            const int64_t begin = origin + chunk.begin;
            const int64_t end = origin + chunk.end;
            const int64_t count = end - begin;
            if (count < 0 || count > capacity) return 0;
            std::fill(grain.begin(), grain.end(), 0.f);
            const int64_t first_input = std::clamp<int64_t>(begin, 0, int64_t(input_frames));
            const int64_t last_input = std::clamp<int64_t>(end, first_input, int64_t(input_frames));
            const size_t copied = size_t(last_input - first_input);
            if (copied && read(context, size_t(first_input), copied, input.data()) != 1) return 0;
            for (size_t frame = 0; frame < copied; ++frame) {
                for (unsigned channel = 0; channel < channels; ++channel)
                    grain[size_t(channel) * capacity + size_t(first_input - begin) + frame] =
                        input[frame * channels + channel];
            }
            const int mute_head = int(std::clamp<int64_t>(-begin, 0, count));
            const int mute_tail = int(std::clamp<int64_t>(end - int64_t(input_frames), 0, count - mute_head));
            dsp.analyseGrain(grain.data(), capacity, mute_head, mute_tail);
            Bungee::OutputChunk rendered{};
            dsp.synthesiseGrain(rendered);
            advance(dsp, request, map, time);
            if (!(request.position > position)) return 0;
            if (!rendered.request[0] || !rendered.request[1] || rendered.frameCount < 0 ||
                rendered.frameCount > 1048576) return 0;
            const double start = rendered.request[0]->position;
            const double stop = rendered.request[1]->position;
            if (!std::isfinite(start) || !std::isfinite(stop) || stop <= start) continue;
            const double skip = std::max(0.0, std::round(-start * rendered.frameCount / (stop - start)));
            const int first = int(std::min<double>(skip, rendered.frameCount));
            const size_t frames = std::min<size_t>(rendered.frameCount - first, output_frames - produced);
            if (frames && (!rendered.data || rendered.channelStride < rendered.frameCount)) return 0;
            if (frames) {
                output.resize(frames * channels);
                for (size_t frame = 0; frame < frames; ++frame) {
                    for (unsigned channel = 0; channel < channels; ++channel)
                        output[frame * channels + channel] =
                            rendered.data[size_t(channel) * rendered.channelStride + first + frame];
                }
                if (write(context, output.data(), frames) != 1) return 0;
            }
            produced += frames;
        }
        return produced == output_frames ? 1 : 0;
    } catch (...) { return 0; }
}
