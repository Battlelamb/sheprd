//! What `fleet scrub` on a host printed: success only when it says its
//! access is gone.

use super::*;

#[test]
fn a_scrub_succeeds_only_when_access_is_known_gone() {
    assert!(remote::scrub_outcome(br#"{"scrubbed":true,"failed":false}"#).is_ok());
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":true,"failed":true}"#),
        Err(Error::Unscrubbed)
    ));
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":false,"failed":true}"#),
        Err(Error::Unscrubbed)
    ));
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":false,"failed":false}"#),
        Err(Error::NoProgram)
    ));
    // An answer that does not say whether anything failed is not trusted.
    assert!(matches!(
        remote::scrub_outcome(br#"{"scrubbed":true}"#),
        Err(Error::Answer(_))
    ));
}
