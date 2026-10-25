// Rill's checked, exception-contained bridge to the unmodified MIT DSP headers.
#include <cstddef>
#include <memory>
#include <random>
#include "signalsmith-stretch.h"

template<class Sample> struct Interleaved {
    Sample *samples;
    int channels;
    struct Channel {
        Sample *samples;
        int stride;
        Sample &operator[](int frame) const { return samples[frame * stride]; }
    };
    Channel operator[](int channel) const { return {samples + channel, channels}; }
};

struct RillStretch {
    // A stable seed makes repeated preparation of one source/rate deterministic.
    signalsmith::stretch::SignalsmithStretch<float, std::mt19937> dsp{0};
    int channels;
};

extern "C" {
void *rill_stretch_create(unsigned channels, unsigned sample_rate, double semitones) noexcept {
    if (channels < 1 || channels > 2 || sample_rate < 8000 || sample_rate > 384000 ||
        !std::isfinite(semitones) || semitones < -24 || semitones > 24)
        return nullptr;
    try {
        auto handle = std::unique_ptr<RillStretch>(new RillStretch);
        handle->channels = int(channels);
        handle->dsp.presetDefault(int(channels), float(sample_rate));
        handle->dsp.setTransposeSemitones(float(semitones));
        return handle.release();
    } catch (...) { return nullptr; }
}
void rill_stretch_destroy(void *handle) noexcept {
    delete static_cast<RillStretch *>(handle);
}
unsigned rill_stretch_input_latency(void *handle) noexcept {
    return unsigned(static_cast<RillStretch *>(handle)->dsp.inputLatency());
}
unsigned rill_stretch_output_latency(void *handle) noexcept {
    return unsigned(static_cast<RillStretch *>(handle)->dsp.outputLatency());
}
int rill_stretch_seek(void *handle, const float *input, unsigned frames, double rate) noexcept {
    if (!handle || !input || frames > 131072 || !std::isfinite(rate) || rate < .25 || rate > 4) return 0;
    try {
        auto &state = *static_cast<RillStretch *>(handle);
        state.dsp.seek(Interleaved<const float>{input, state.channels}, int(frames), rate);
        return 1;
    } catch (...) { return 0; }
}
int rill_stretch_process(void *handle, const float *input, unsigned input_frames,
                        float *output, unsigned output_frames) noexcept {
    if (!handle || !input || !output || input_frames > 16384 || output_frames > 4096)
        return 0;
    try {
        auto &state = *static_cast<RillStretch *>(handle);
        state.dsp.process(Interleaved<const float>{input, state.channels}, int(input_frames),
                          Interleaved<float>{output, state.channels}, int(output_frames));
        return 1;
    } catch (...) { return 0; }
}
int rill_stretch_flush(void *handle, float *output, unsigned frames) noexcept {
    if (!handle || !output || frames > 131072) return 0;
    try {
        auto &state = *static_cast<RillStretch *>(handle);
        state.dsp.flush(Interleaved<float>{output, state.channels}, int(frames));
        return 1;
    } catch (...) { return 0; }
}
}
