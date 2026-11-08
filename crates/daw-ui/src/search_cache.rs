//! Retain result indices without copying borrowed catalogues into UI state.
use std::sync::Arc;

#[derive(Clone)]
struct SearchCache {
    signature: u64,
    indices: Arc<[usize]>,
}

/// The caller hashes all fields that affect matching/order. Row values stay
/// live, and unchanged frames only clone an Arc instead of rebuilding results.
pub(crate) fn cached_indices(
    ctx: &egui::Context,
    id: egui::Id,
    signature: u64,
    compute: impl FnOnce() -> Vec<usize>,
) -> Arc<[usize]> {
    if let Some(cached) = ctx.data(|data| data.get_temp::<SearchCache>(id))
        && cached.signature == signature
    {
        return cached.indices;
    }
    let indices: Arc<[usize]> = compute().into();
    ctx.data_mut(|data| {
        data.insert_temp(
            id,
            SearchCache {
                signature,
                indices: Arc::clone(&indices),
            },
        );
    });
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_reuses_indices_and_recomputes_only_when_signature_changes() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("catalogue");
        let first = cached_indices(&ctx, id, 1, || vec![2, 0]);
        let again = cached_indices(&ctx, id, 1, || panic!("unchanged search recomputed"));
        assert!(Arc::ptr_eq(&first, &again));
        let changed = cached_indices(&ctx, id, 2, || vec![1]);
        assert_eq!(&*changed, &[1]);
        assert_eq!(&*first, &[2, 0]);
        let separate = cached_indices(&ctx, id.with("other"), 2, Vec::new);
        assert!(separate.is_empty());
    }
}
