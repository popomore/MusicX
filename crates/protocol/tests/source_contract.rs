use musicx_protocol::{MusicSourceAction, SourceCapability, SourcePageRequest};
use serde_json::json;

#[test]
fn source_page_requests_keep_legacy_wire_names_and_reject_script_exports() {
    for (wire, expected) in [
        (
            json!({"action":"page","source":"independent:fixture","method":"home","params":{}}),
            SourcePageRequest::Recommendations {},
        ),
        (
            json!({"action":"page","source":"independent:fixture","method":"home"}),
            SourcePageRequest::Recommendations {},
        ),
        (
            json!({"action":"page","source":"independent:fixture","method":"home","params":null}),
            SourcePageRequest::Recommendations {},
        ),
        (
            json!({"action":"page","source":"independent:fixture","method":"browse","params":{"page":"opaque:page/one"}}),
            SourcePageRequest::Browse {
                page: "opaque:page/one".into(),
            },
        ),
    ] {
        let action: MusicSourceAction = serde_json::from_value(wire).unwrap();
        let round_trip: MusicSourceAction =
            serde_json::from_value(serde_json::to_value(&action).unwrap()).unwrap();
        assert!(
            matches!(&round_trip, MusicSourceAction::Page { request, .. } if request == &expected)
        );
        let MusicSourceAction::Page { request, .. } = action else {
            panic!("expected native page")
        };
        assert_eq!(request, expected);
    }
    for wire in [
        json!({"method":"postComment","params":{"text":"must not run"}}),
        json!({"method":"browse","params":{"page":true}}),
        json!({"method":"playlist","params":{}}),
    ] {
        assert!(serde_json::from_value::<SourcePageRequest>(wire).is_err());
    }
    assert_eq!(
        serde_json::to_value(SourceCapability::Playback).unwrap(),
        "stream"
    );
    assert_eq!(
        serde_json::from_value::<SourceCapability>(json!("home")).unwrap(),
        SourceCapability::Recommendations
    );
    assert!(serde_json::from_value::<SourceCapability>(json!("remotePlay")).is_err());
    assert!(serde_json::from_value::<SourceCapability>(json!("privateSessionHelper")).is_err());
}
