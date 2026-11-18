use wasm_bindgen::prelude::*;

mod connectivity;
mod output;
mod pipeline;
mod raster;
mod roost;

pub use connectivity::*;
pub use pipeline::{run_resistance_pipeline_browser, BrowserResistanceOutput, NamedInputArrays};
pub use raster::*;
pub use roost::roost_finder_compute;

#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}
