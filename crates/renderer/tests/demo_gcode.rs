//! The demo asset must exercise every curve kind the renderer draws.

use gcode_viewer::{MoveKind, motions};

#[test]
fn the_demo_asset_covers_every_move_kind() {
    let source = include_str!("../../../ui/public/demo.gcode");

    let records = motions(source);

    assert!(records.iter().any(|r| r.kind == MoveKind::Rapid));
    assert!(records.iter().any(|r| r.kind == MoveKind::Feed));
    assert!(
        records
            .iter()
            .any(|r| matches!(r.kind, MoveKind::Arc { .. }))
    );
    assert!(records.iter().any(|r| r.kind == MoveKind::Spline));
}

#[test]
fn the_demo_curves_carry_their_offsets_and_endpoints() {
    let source = include_str!("../../../ui/public/demo.gcode");

    let curves: Vec<_> = motions(source)
        .into_iter()
        .filter(|r| !matches!(r.kind, MoveKind::Rapid | MoveKind::Feed))
        .collect();

    assert!(
        curves.len() >= 5,
        "expected several curves: {}",
        curves.len()
    );
    for curve in curves {
        assert!(curve.curve.is_some(), "{}", curve.line);
        assert!(curve.start.is_some(), "{}", curve.line);
        assert!(curve.end.is_some(), "{}", curve.line);
    }
}
