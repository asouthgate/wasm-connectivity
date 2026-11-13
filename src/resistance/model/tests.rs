use super::interval::Interval;
use super::*;
use crate::resistance::distance::distance_generation;
use crate::resistance::pipeline::{run_resistance_pipeline, ResistanceParams};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

fn example() -> Model {
    serde_json::from_str(include_str!("../../../example/horseshoe-bat-model.json")).unwrap()
}
fn grid() -> Grid {
    Grid {
        nrows: 1,
        ncols: 6,
        pixel_width: 2.0,
    }
}
fn small_model(operations: Value) -> Model {
    serde_json::from_value(json!({
      "name": "test",
      "inputs": {
        "data": {
          "type": "raster_ranked"
        }
      },
      "nodes": {
        "result": {
          "input": "inputs.data",
          "operations": operations
        }
      },
      "output": "nodes.result"
    }))
    .unwrap()
}
fn run_small(model: &Model, values: Vec<f64>) -> Result<Vec<f64>> {
    model.run(
        HashMap::from([("data".into(), PipelineData::raster(values))]),
        grid(),
    )
}
fn assert_same(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
        assert!(
            a.is_nan() && b.is_nan() || (a - b).abs() <= 1e-12 * b.abs().max(1.0),
            "cell {i}: {a} != {b}"
        );
    }
}

#[test]
fn raycaster_outside_points_do_not_contribute_or_error() {
    let model: Model = serde_json::from_value(serde_json::json!({
        "name": "lights", "inputs": {
            "terrain": {"type": "raster_topography"},
            "buildings": {"type": "vector_occluder"},
            "lights": {"type": "raycaster_point_source"}
        },
        "nodes": {"light": {
            "op": "irradiance", "inputs": {
                "point_sources": "inputs.lights",
                "topography": "inputs.terrain",
                "opaque_surface": "inputs.buildings",
                "translucent_surface": "inputs.buildings"
            },
            "parameters": {"cutoff_metres": 10, "sensor_height_metres": 0, "absorption": 0.5}
        }}, "output": "nodes.light"
    }))
    .unwrap();
    let mut fields = HashMap::from([
        ("terrain".into(), PipelineData::raster(vec![0.0; 4])),
        ("buildings".into(), PipelineData::raster(vec![0.0; 4])),
        (
            "lights".into(),
            PipelineData::Points(Arc::new(vec![
                -1.0, 0.0, 3.0, 2.0, 0.0, 3.0, 0.0, -1.0, 3.0, 0.0, 2.0, 3.0,
            ])),
        ),
    ]);
    let grid = Grid {
        nrows: 2,
        ncols: 2,
        pixel_width: 1.0,
    };
    assert_eq!(model.run(fields.clone(), grid).unwrap(), vec![0.0; 4]);
    fields.insert(
        "lights".into(),
        PipelineData::Points(Arc::new(vec![-1.0, 0.0, -3.0])),
    );
    assert!(model
        .run(fields, grid)
        .unwrap_err()
        .contains("invalid point coordinates or height"));
}

#[test]
fn interval_mapping_respects_endpoints_unmatched_and_missing() {
    let model = small_model(json!([{
      "op": "discrete_map",
      "parameters": {
        "intervals": {
          "(1,3)": 4,
          "(3,6)": 1,
          "[6,inf)": 2
        },
        "unmatched": 0
      }
    }]));
    assert_same(
        &run_small(&model, vec![1.0, 2.0, 3.0, 4.0, 6.0, f64::NAN]).unwrap(),
        &[0.0, 4.0, 0.0, 1.0, 2.0, f64::NAN],
    );
    for (text, expected) in [
        ("[1,3)", vec![1.0, 2.0]),
        ("(3,5]", vec![4.0, 5.0]),
        ("[4,4]", vec![4.0]),
    ] {
        let interval = Interval::parse(text).unwrap();
        let actual: Vec<f64> = (0..7)
            .map(|v| v as f64)
            .filter(|v| interval.contains(*v))
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn invalid_intervals_and_overlaps_are_errors() {
    for text in [
        "[3,1]",
        "(3,3)",
        "[inf,inf]",
        "[-inf,1]",
        "[1,2",
        "[NaN,2]",
        "[1,2,3]",
        "é1,3)",
    ] {
        assert!(Interval::parse(text).is_err(), "{text}");
    }
    for intervals in [
        json!({
          "[1,3]": 1,
          "[3,5]": 2
        }),
        json!({
          "(0,10)": 1,
          "[2,3]": 2
        }),
    ] {
        let model = small_model(json!([{
          "op": "discrete_map",
          "parameters": {
            "intervals": intervals,
            "unmatched": 0
          }
        }]));
        let error = run_small(&model, vec![0.0; 6]).unwrap_err();
        assert!(
            error.contains("node result: overlapping interval"),
            "{error}"
        );
    }
}

#[test]
fn unmatched_error_and_nodata_are_distinct() {
    let op = json!({
      "op": "discrete_map",
      "parameters": {
        "intervals": {
          "[1,1]": 2
        },
        "unmatched": "error"
      }
    });
    let model = small_model(json!([op]));
    assert!(run_small(&model, vec![3.0; 6])
        .unwrap_err()
        .contains("unmatched value 3 at cell 0"));
    let model = small_model(json!([{
      "op": "discrete_map",
      "parameters": {
        "intervals": {},
        "unmatched": "nodata"
      }
    }]));
    assert!(run_small(&model, vec![3.0; 6])
        .unwrap()
        .iter()
        .all(|v| v.is_nan()));
}

#[test]
fn distance_response_preserves_boundary_empty_and_missing_rules() {
    let model = small_model(json!([{
      "op": "distance",
      "parameters": {
        "units": "metres",
        "when_empty": "infinity"
      }
    }, {
      "op": "power_response",
      "parameters": {
        "input_range": [0, 4],
        "base_range": [1, 0.5],
        "exponent": 2,
        "scale": 16,
        "offset": 0,
        "above": 0
      }
    }]));
    assert_same(
        &run_small(&model, vec![1.0, 0.0, 0.0, 0.0, f64::NAN, 0.0]).unwrap(),
        &[16.0, 9.0, 4.0, 0.0, f64::NAN, 0.0],
    );
    assert_same(
        &run_small(&model, vec![0.0, 0.0, 0.0, 0.0, f64::NAN, 0.0]).unwrap(),
        &[0.0, 0.0, 0.0, 0.0, f64::NAN, 0.0],
    );
}

#[test]
fn invalid_responses_references_cycles_and_types_are_errors() {
    for response in [
        json!({
          "op": "power_response",
          "parameters": {
            "input_range": [0, 0],
            "base_range": [0, 1],
            "exponent": 1,
            "scale": 1,
            "offset": 0
          }
        }),
        json!({
          "op": "power_response",
          "parameters": {
            "input_range": [0, 1],
            "base_range": [0, 1],
            "exponent": 0,
            "scale": 1,
            "offset": 0
          }
        }),
        json!({
          "op": "power_response",
          "parameters": {
            "input_range": [0, 1],
            "base_range": [0, 1],
            "exponent": 1,
            "scale": 1,
            "offset": 0
          }
        }),
        json!({
          "op": "power_response",
          "parameters": {
            "input_range": [0, 10],
            "base_range": [-1, -1],
            "exponent": 0.5,
            "scale": 1,
            "offset": 0
          }
        }),
    ] {
        assert!(run_small(&small_model(json!([response])), vec![2.0; 6]).is_err());
    }
    let mut model = small_model(json!([]));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Chain {
            input: Reference::Input {
                from_input: "missing".into(),
            },
            operations: vec![],
        },
    );
    assert!(run_small(&model, vec![0.0; 6])
        .unwrap_err()
        .contains("unknown input: missing"));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Chain {
            input: Reference::node("result".into()),
            operations: vec![],
        },
    );
    assert!(run_small(&model, vec![0.0; 6])
        .unwrap_err()
        .contains("cyclic node reference"));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Node(Node {
            operation: Operation::Sum {},
            inputs: BTreeMap::from([("data".into(), InputEdges::Many(vec![]))]),
        }),
    );
    assert!(run_small(&model, vec![0.0; 6])
        .unwrap_err()
        .contains("at least one reference"));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Node(Node {
            operation: Operation::Round,
            inputs: BTreeMap::new(),
        }),
    );
    assert!(run_small(&model, vec![0.0; 6])
        .unwrap_err()
        .contains("missing input port data"));
    model = small_model(json!([]));
    assert!(model
        .run(
            HashMap::from([("data".into(), PipelineData::raster(vec![0.0; 5]))]),
            grid()
        )
        .is_err());
    model
        .inputs
        .insert("data".into(), Input::RaycasterPointSource {});
    assert!(model
        .run(
            HashMap::from([("data".into(), PipelineData::Points(Arc::new(vec![])))]),
            grid()
        )
        .is_err());
}

#[test]
fn model_schema_rejects_unknown_operations_fields_and_policies() {
    let original: Value =
        serde_json::from_str(include_str!("../../../example/horseshoe-bat-model.json")).unwrap();
    for (input, alias) in [("buildings", "value_field"), ("lamps", "height_field")] {
        let mut value = original.clone();
        value["inputs"][input][alias] = json!("value");
        assert!(serde_json::from_value::<Model>(value).is_err());
    }
    for (input, key, setting) in [
        ("roads", "required_fields", json!(["presence"])),
        ("buildings", "background", json!(0)),
    ] {
        let mut value = original.clone();
        value["inputs"][input][key] = setting;
        assert!(serde_json::from_value::<Model>(value).is_err());
    }
    for (pointer, value) in [
        ("/nodes/road_res/operations/0/op", json!("script")),
        (
            "/nodes/tree_res/operations/1/parameters/units",
            json!("feet"),
        ),
        (
            "/nodes/tree_res/operations/1/parameters/when_empty",
            json!("zero"),
        ),
        ("/nodes/road_res/operations/0/typo", json!(1)),
    ] {
        let mut value_copy = original.clone();
        if let Some(slot) = value_copy.pointer_mut(pointer) {
            *slot = value;
        } else {
            value_copy["nodes"]["road_res"]["operations"][0]["typo"] = value;
        }
        assert!(
            serde_json::from_value::<Model>(value_copy).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn horseshoe_spec_matches_every_existing_resistance_layer() {
    let (m, n) = (13, 17);
    let total = m * n;
    for pixel_width in [1.0, 2.0, 5.0] {
        for scenario in 0..4 {
            let mut dtm = vec![100.0; total];
            let heights = [0.0, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0];
            let mut dsm: Vec<f64> = (0..total)
                .map(|i| 100.0 + heights[i % heights.len()])
                .collect();
            let mut buildings = vec![0.0; total];
            let mut lcm: Vec<f64> = (0..total).map(|i| (i % 23) as f64).collect();
            let mut roads = vec![0.0; total];
            let mut rivers = vec![0.0; total];
            let mut generic = vec![3.0; total];
            let lamps = if scenario == 0 {
                vec![]
            } else {
                vec![4.0, 5.0, 8.0, 12.0, 8.0, 5.0]
            };
            if scenario > 0 {
                roads[0] = 1.0;
                roads[total - 1] = 2.0;
                rivers[n + 2] = 1.0;
            }
            if scenario == 2 {
                roads.fill(1.0);
                rivers.fill(1.0);
            }
            if scenario == 3 {
                dsm.fill(102.0);
            }
            buildings[10] = 5.0;
            buildings[11] = 4.0;
            buildings[12] = 2.0;
            dsm[10] = f64::NAN;
            dtm[11] = f64::NAN;
            lcm[12] = f64::NAN;
            dtm[13] = f64::NAN;
            dsm[14] = f64::NAN;
            roads[15] = f64::NAN;
            rivers[16] = f64::NAN;
            generic[17] = f64::NAN;
            let params = ResistanceParams {
                road_buffer: 200.0,
                road_resmax: 10.0,
                road_xmax: 5.0,
                river_buffer: 10.0,
                river_resmax: 2000.0,
                river_xmax: 4.0,
                landscape_rankmax: 8.0,
                landscape_resmax: 100.0,
                landscape_xmax: 5.0,
                linear_buffer: 10.0,
                linear_rankmax: 4.0,
                linear_resmax: 22000.0,
                linear_xmax: 3.0,
                lamp_resmax: 1e8,
                lamp_xmax: 1.0,
                lamp_ext: 100.0,
                pixw: pixel_width,
                nrows: m,
                ncols: n,
            };
            let expected = run_resistance_pipeline(
                &roads,
                &rivers,
                &buildings,
                Some(&lcm),
                &dtm,
                &dsm,
                &generic,
                &lamps,
                &params,
                None,
                None,
            );
            let mut fields = HashMap::new();
            for (name, values) in [
                ("dtm", dtm),
                ("dsm", dsm),
                ("landcover", lcm),
                ("buildings", buildings),
                ("roads", roads),
                ("rivers", rivers),
                ("generic", generic),
            ] {
                let mut values = values;
                if name == "roads" || name == "rivers" {
                    values = distance_generation(&values, m, n, f64::INFINITY);
                    for value in &mut values {
                        *value *= pixel_width;
                    }
                }
                let field = PipelineData::raster(values);
                fields.insert(name.into(), field);
            }
            fields.insert("lamps".into(), PipelineData::Points(Arc::new(lamps)));
            let mut model = example();
            for (name, expected) in [
                ("road_res", expected.road_res),
                ("river_res", expected.river_res),
                ("landscape_res", expected.landscape_res),
                ("linear_res", expected.linear_res),
                ("lamp_res", expected.lamp_res),
                ("total_res", expected.total_res),
                ("surfaces.translucent", expected.soft_surf),
                ("surfaces.opaque", expected.hard_surf),
            ] {
                model.output = if let Some((node, port)) = name.split_once('.') {
                    Reference::Node {
                        from_node: node.into(),
                        output: Some(port.into()),
                    }
                } else {
                    Reference::node(name.into())
                };
                let actual = model
                    .run(
                        fields.clone(),
                        Grid {
                            nrows: m,
                            ncols: n,
                            pixel_width,
                        },
                    )
                    .unwrap();
                assert_same(&actual, &expected);
            }
        }
    }
}
#[test]
fn canonical_nodes_and_compact_chains_execute_the_same_graph() {
    let chain = small_model(json!([
        {"op":"discrete_map","parameters":{"intervals":{"[0,2]":2,"[3,4]":4},"unmatched":"error"}},
        {"op":"power_response","parameters":{"input_range":[0,4],"base_range":[0,1],"exponent":2,"scale":100,"offset":0}}
    ]));
    let explicit:Model=serde_json::from_value(json!({
        "name":"explicit","inputs":{"data":{"type":"raster_ranked"}},
        "nodes":{
            "a_response":{"op":"power_response","inputs":"nodes.z_map",
                "parameters":{"input_range":[0,4],"base_range":[0,1],"exponent":2,"scale":100,"offset":0}},
            "z_map":{"op":"discrete_map","inputs":"inputs.data",
                "parameters":{"intervals":{"[0,2]":2,"[3,4]":4},"unmatched":"error"}}
        },"output":"nodes.a_response"
    })).unwrap();
    let values = vec![0.0, 1.0, 2.0, 3.0, 4.0, f64::NAN];
    let expected = [25.0, 25.0, 25.0, 100.0, 100.0, f64::NAN];
    assert_same(&run_small(&chain, values.clone()).unwrap(), &expected);
    assert_same(&run_small(&explicit, values).unwrap(), &expected);
    let graph = chain.compile().unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.nodes["result"].inputs.len(), 1);
}

#[test]
fn graph_checks_named_ports_types_units_and_cycles_without_loading_data() {
    let mut model = example();
    model.output = Reference::node("surfaces".into());
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("select a declared output port"));
    model.output = Reference::Node {
        from_node: "surfaces".into(),
        output: Some("opaque".into()),
    };
    assert!(model.compile().is_ok());
    model.output = Reference::Node {
        from_node: "surfaces".into(),
        output: Some("missing".into()),
    };
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("has no output missing"));
    let mut model = example();
    let NodeSpec::Node(node) = model.nodes.get_mut("irradiance").unwrap() else {
        panic!()
    };
    node.inputs.insert(
        "point_sources".into(),
        InputEdges::One(Reference::Input {
            from_input: "dtm".into(),
        }),
    );
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("point_sources must be raycaster_point_source"));
    let mut model = example();
    let NodeSpec::Node(node) = model.nodes.get_mut("total_res").unwrap() else {
        panic!()
    };
    node.inputs.insert(
        "data".into(),
        InputEdges::Many(vec![
            Reference::Input {
                from_input: "roads".into(),
            },
            Reference::Input {
                from_input: "generic".into(),
            },
        ]),
    );
    assert!(model.compile().err().unwrap().contains("same units"));
    let mut model = small_model(json!([]));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Node(Node {
            operation: Operation::Round,
            inputs: BTreeMap::from([(
                "data".into(),
                InputEdges::Many(vec![Reference::Input {
                    from_input: "data".into(),
                }]),
            )]),
        }),
    );
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("expects one reference"));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Node(Node {
            operation: Operation::Round,
            inputs: BTreeMap::from([
                (
                    "data".into(),
                    InputEdges::One(Reference::Input {
                        from_input: "data".into(),
                    }),
                ),
                (
                    "typo".into(),
                    InputEdges::One(Reference::Input {
                        from_input: "data".into(),
                    }),
                ),
            ]),
        }),
    );
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("unknown input port typo"));
    model.nodes.insert(
        "result".into(),
        NodeSpec::Chain {
            input: Reference::node("other".into()),
            operations: vec![],
        },
    );
    model.nodes.insert(
        "other".into(),
        NodeSpec::Chain {
            input: Reference::node("result".into()),
            operations: vec![],
        },
    );
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("cyclic node reference"));
}

#[test]
fn references_and_parameters_are_not_ambiguous_strings_or_flat_options() {
    for definition in [
        json!({"op":"round", "inputs":{"data":"inputs.data"}}),
        json!({"op":"sum", "inputs":{"data":["inputs.data"]}}),
    ] {
        assert!(serde_json::from_value::<Node>(definition).is_err());
    }
    for invalid in [
        "data",
        "inputs.",
        "inputs.roads.extra",
        "nodes.",
        "nodes.surfaces.",
        "nodes.surfaces.opaque.extra",
        "other.roads",
    ] {
        assert!(
            serde_json::from_value::<Reference>(json!(invalid)).is_err(),
            "{invalid}"
        );
    }
    for invalid in [
        json!({"from_input": "roads"}),
        json!({"from_node": "road_res"}),
    ] {
        assert!(serde_json::from_value::<Reference>(invalid).is_err());
    }
    let reference: Reference = serde_json::from_value(json!("nodes.surfaces.opaque")).unwrap();
    assert!(reference.selects_node("surfaces", "opaque"));
    let reference: Reference = serde_json::from_value(json!("nodes.road_res")).unwrap();
    assert!(reference.selects_node("road_res", "data"));
    assert!(
        matches!(serde_json::from_value::<Reference>(json!("inputs.roads")).unwrap(), Reference::Input { from_input } if from_input == "roads")
    );
    assert!(
        serde_json::from_value::<Reference>(json!({"from_input":"data","from_node":"other"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<Node>(json!({"op":"round","inputs":"inputs.data","typo":0}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<Node>(json!({"op":"round","parameters":{"op":"sum"}})).is_err()
    );
    let model = small_model(
        json!([{"op":"power_response","parameters":{"input_range":[0,0],"base_range":[0,1],"exponent":2,"scale":1,"offset":0}}]),
    );
    assert!(model
        .compile()
        .err()
        .unwrap()
        .contains("invalid power response parameters"));
}
