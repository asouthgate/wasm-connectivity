//! Single-slot, single-threaded warm-start cache for interactive
//! connectivity analysis in the browser.
//!
//! Circuitscape always rebuilds the circuit model from scratch for each run.
//! In contrast, the typical interactive web-map workflow only changes the
//! source/ground rasters between solves, while the resistance raster stays
//! fixed; or it makes small edits to the resistance raster that should not
//! require a fully cold PCG restart. This module caches the expensive
//! artifacts of `build_circuit_model` (cell_to_node, laplacian, connected
//! components) plus the most recent per-node voltage vector, so a re-solve
//! can either:
//!   * skip the Laplacian rebuild entirely (resistance unchanged), or
//!   * rebuild the Laplacian but seed PCG with the prior voltage solution.
//!
//! The cache uses a `thread_local!` single slot: WebAssembly is single
//! threaded, so there is no key, no map, no eviction, and no `Mutex`. The
//! caller is expected to call `reset()` whenever the resistance raster is
//! structurally changed (different dimensions or different nodata sentinel),
//! and is free to call `reset()` at any other time to force a fresh build.

use sprs::CsMat;

/// In-memory cache of the resistance-derived circuit artifacts plus the
/// most recent computed voltage field.
pub struct BuildCache {
    pub laplacian: CsMat<f64>,
    pub cell_to_node: Vec<i32>,
    pub num_nodes: usize,
    pub components: Vec<Vec<usize>>,
    pub nrows: usize,
    pub ncols: usize,
    pub nodata: f64,
    /// Most recent per-node voltage vector computed over this circuit. Used
    /// as a PCG initial guess when those same nodes are re-solved. May be
    /// empty if no solve has been performed yet.
    pub last_voltages: Vec<f64>,
}

thread_local! {
    static CACHE: std::cell::RefCell<Option<BuildCache>> = std::cell::RefCell::new(None);
}

/// Stores a freshly built circuit model into the cache, replacing any
/// previous contents.
///
/// To support warm starts across `obtain_circuit`'s take-and-re-store
/// dance, any previously cached per-node voltage field is preserved if
/// and only if the new circuit has the same dimensions and nodata
/// sentinel as the old one. If the circuit identity changes the prior
/// voltages could not have been indexed the same way, so they are
/// dropped (which is also what happens on a first-ever `store`).
pub fn store(
    laplacian: CsMat<f64>,
    cell_to_node: Vec<i32>,
    num_nodes: usize,
    components: Vec<Vec<usize>>,
    nrows: usize,
    ncols: usize,
    nodata: f64,
) {
    CACHE.with(|c| {
        let preserved = c.borrow().as_ref().and_then(|b| {
            if (b.nrows, b.ncols, b.nodata, b.last_voltages.len()) == (nrows, ncols, nodata, num_nodes) {
                Some(b.last_voltages.clone())
            } else {
                None
            }
        });
        *c.borrow_mut() = Some(BuildCache {
            laplacian,
            cell_to_node,
            num_nodes,
            components,
            nrows,
            ncols,
            nodata,
            last_voltages: preserved.unwrap_or_default(),
        });
    });
}

/// Takes ownership of the cached model, leaving the slot empty.
pub fn take() -> Option<BuildCache> {
    CACHE.with(|c| c.borrow_mut().take())
}

/// Returns a copy of the cached dimensions and nodata sentinel, if any.
/// Useful for callers deciding whether the cached circuit is still valid
/// for the resistance raster they are about to solve.
pub fn peek_meta() -> Option<(usize, usize, f64)> {
    CACHE.with(|c| {
        c.borrow().as_ref().map(|b| (b.nrows, b.ncols, b.nodata))
    })
}

/// Replaces the cached last-voltage vector. No-op if no cache is present.
pub fn store_last_voltages(voltages: &[f64]) {
    CACHE.with(|c| {
        if let Some(b) = c.borrow_mut().as_mut() {
            b.last_voltages.clear();
            b.last_voltages.extend_from_slice(voltages);
        }
    });
}

/// Returns a clone of the cached last-voltage vector, if any.
pub fn last_voltages() -> Vec<f64> {
    CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .map(|b| b.last_voltages.clone())
            .unwrap_or_default()
    })
}

/// Drops any cached circuit model. Safe to call when the cache is empty.
pub fn reset() {
    CACHE.with(|c| *c.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_take_roundtrip() {
        reset();
        assert!(take().is_none());

        let (_cell_to_node, _num_nodes, _edges, lap, comps) =
            crate::build_circuit_model(&[1.0; 9], 3, 3, crate::NODATA_SENTINEL);
        store(lap.clone(), vec![1, 2, 3, 0], 3, comps.clone(), 3, 3, -9999.0);
        assert_eq!(peek_meta(), Some((3, 3, -9999.0)));

        let taken = take().expect("cache should be populated");
        assert_eq!(taken.nrows, 3);
        assert_eq!(taken.ncols, 3);
        assert_eq!(taken.cell_to_node, vec![1, 2, 3, 0]);
        assert!(take().is_none(), "take should drain the slot");
        reset();
    }

    #[test]
    fn test_last_voltages_roundtrip() {
        reset();
        store_last_voltages(&[1.0, 2.0, 3.0]);
        assert!(last_voltages().is_empty(), "no circuit stored -> no voltages");

        let (_cell_to_node, _n, _e, lap, comps) =
            crate::build_circuit_model(&[1.0; 4], 2, 2, crate::NODATA_SENTINEL);
        store(lap, vec![1, 2, 3, 4], 4, comps, 2, 2, -9999.0);
        store_last_voltages(&[10.0, 20.0, 30.0, 40.0]);
        assert_eq!(last_voltages(), vec![10.0, 20.0, 30.0, 40.0]);
        reset();
        assert!(last_voltages().is_empty());
    }
}