//! Each link of the certificate chain is checked as `ios/Spikes/attest-check.py`
//! checks it: the names link, each issuer may sign at its depth, and the
//! leaf is not a CA.

use super::attest::{INTERMEDIATE, Pki, Shape, ca, verify};
use crate::attest::Error;
use rcgen::{BasicConstraints, DnType, IsCa, KeyUsagePurpose};

fn unconstrained() -> IsCa {
    IsCa::Ca(BasicConstraints::Unconstrained)
}

#[test]
fn an_intermediate_with_key_usage_that_allows_signing_certificates_is_taken() {
    let mut intermediate = ca(INTERMEDIATE, true);
    intermediate.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let pki = Pki::with(IsCa::Ca(BasicConstraints::Constrained(1)), intermediate);
    assert!(verify(&pki, &Shape::default()).is_ok());
}

#[test]
fn an_intermediate_whose_key_usage_lacks_certificate_signing_is_refused() {
    let mut intermediate = ca(INTERMEDIATE, true);
    intermediate.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let pki = Pki::with(unconstrained(), intermediate);
    assert_eq!(verify(&pki, &Shape::default()), Err(Error::Chain));
}

#[test]
fn a_root_whose_path_length_forbids_the_intermediate_is_refused() {
    let pki = Pki::with(
        IsCa::Ca(BasicConstraints::Constrained(0)),
        ca(INTERMEDIATE, true),
    );
    assert_eq!(verify(&pki, &Shape::default()), Err(Error::Chain));
}

#[test]
fn a_leaf_that_is_a_ca_is_refused() {
    let shape = Shape {
        leaf_ca: true,
        ..Shape::default()
    };
    assert_eq!(verify(&Pki::new(), &shape), Err(Error::Chain));
}

#[test]
fn a_leaf_whose_issuer_is_not_the_intermediate_is_refused() {
    // The leaf is still signed by the intermediate's key; only its issuer
    // name differs from the intermediate's subject.
    let mut pki = Pki::new();
    pki.intermediate.distinguished_name = rcgen::DistinguishedName::new();
    pki.intermediate
        .distinguished_name
        .push(DnType::CommonName, "Another CA");
    assert_eq!(verify(&pki, &Shape::default()), Err(Error::Chain));
}
