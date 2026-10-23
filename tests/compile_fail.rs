//! The invariants the type system holds: a blob hash, an authorization, a report and a payment
//! statement exist only once they were checked. Each file under `compile_fail/` is a construction
//! that must not build, kept beside the compiler's reason for refusing it. A `compile_fail`
//! doctest would pass on any error, including a renamed import.
//!
//! After a deliberate change or a toolchain bump, regenerate the reasons with
//! `TRYBUILD=overwrite cargo test --test compile_fail`.

#[test]
fn checked_values_cannot_be_built_without_their_checks() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
