use wasm_bindgen::prelude::*;

mod connectivity;
mod output;
mod pipeline;
mod raster;
mod roost;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
mod threads;

pub use connectivity::*;
pub use pipeline::{run_resistance_pipeline_browser, BrowserResistanceOutput, NamedInputArrays};
pub use raster::*;
pub use roost::roost_finder_compute;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use threads::{initialize_pipeline_threads, run_pipeline_thread};

#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}
