// Output serialization and precision conversion for the WASM interface.
use crate::solve;
use serde::Serialize;
use serde_json::json;

pub(crate) fn json_response<T: Serialize>(output: &T) -> String {
    serde_json::to_string(output).unwrap_or_else(|e| {
        serde_json::to_string(&json!({ "error": e.to_string() }))
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
    })
}

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_CHARS[((triple >> 6) & 0x3F) as usize] as char
        } else {
            b'=' as char
        });
        out.push(if chunk.len() > 2 {
            BASE64_CHARS[(triple & 0x3F) as usize] as char
        } else {
            b'=' as char
        });
    }
    out
}

/// Round f64 values to f32, then encode their little-endian bytes as Base64.
pub(crate) fn encode_f64_as_f32_base64(data: &[f64]) -> String {
    let mut bytes = vec![0u8; data.len() * 4];
    for (i, &v) in data.iter().enumerate() {
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&(v as f32).to_le_bytes());
    }
    encode_base64(&bytes)
}

// Call after current calculation and convergence checks have completed in f64.
// Round the output to f32, reusing supplied buffers instead of allocating maps.
pub(crate) fn maps_to_f32_reusing_buffers(
    output: solve::RasterOutput<f64>,
    mut voltage: Vec<f32>,
    mut current: Vec<f32>,
) -> solve::RasterOutput<f32> {
    assert_eq!(voltage.len(), output.voltages.len());
    assert_eq!(current.len(), output.current_map.len());
    for (dest, &value) in voltage.iter_mut().zip(&output.voltages) {
        *dest = value as f32;
    }
    for (dest, &value) in current.iter_mut().zip(&output.current_map) {
        *dest = value as f32;
    }
    solve::RasterOutput {
        voltages: voltage,
        current_map: current,
        nrows: output.nrows,
        ncols: output.ncols,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linalg;

    #[test]
    fn mixed_browser_maps_reuse_f32_buffers_after_f64_current_calculation() {
        let mut source = vec![1.0_f32, 0.0, 0.0];
        let ground = vec![0.0_f32, 0.0, 1.0];
        let voltage_ptr = source.as_ptr();
        let current_ptr = ground.as_ptr();
        let result = solve::mixed::solve_raster_sources_mg_mixed_with_options(
            &[1e-6_f32; 3],
            1,
            3,
            -9999.0,
            &mut source,
            &ground,
            1000,
            1e-3,
            false,
            solve::GroundMode::Shunt,
            linalg::multigrid::MgOptions::default(),
        );
        assert!(result.error.is_none());
        let expected_voltage: Vec<f32> = result.output.voltages.iter().map(|&v| v as f32).collect();
        let expected_current: Vec<f32> = result
            .output
            .current_map
            .iter()
            .map(|&v| v as f32)
            .collect();
        let compact = maps_to_f32_reusing_buffers(result.output, source, ground);
        assert_eq!(compact.voltages.as_ptr(), voltage_ptr);
        assert_eq!(compact.current_map.as_ptr(), current_ptr);
        assert_eq!(compact.voltages, expected_voltage);
        assert_eq!(compact.current_map, expected_current);
        for &current in &compact.current_map {
            assert!((current - 1.0).abs() < 1e-3);
        }
        // Recomputing from rounded output voltages would lose the current accuracy.
        let rounded_current = solve::current::compute_current_map_from_raster(
            &[1e-6_f32; 3],
            &[0.0_f32, 0.0, 1.0],
            &compact.voltages,
            1,
            3,
            -9999.0,
            true,
        );
        assert!(rounded_current.iter().any(|&v| (v - 1.0).abs() > 0.01));
    }
}
