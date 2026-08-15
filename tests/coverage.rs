use oxidtr::backend::coverage::{Coverage, ElementKind, Verification};

/// The manifest carries a header that names every status, so a test asking
/// "does `declined` appear?" must look at the entries, not the whole text.
fn entry_lines(c: &Coverage) -> Vec<String> {
    c.render().lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The manifest is an artifact `check` reads and a human reviews in a diff, so
/// its byte layout must not depend on the order the backend happened to visit
/// the model in.
#[test]
fn rendering_is_deterministic_regardless_of_record_order() {
    let mut a = Coverage::new();
    a.record(ElementKind::Assert, "Zeta", Verification::Verified);
    a.record(ElementKind::Fact, "Alpha", Verification::Verified);
    a.record(ElementKind::Fact, "Beta", Verification::ByType);

    let mut b = Coverage::new();
    b.record(ElementKind::Fact, "Beta", Verification::ByType);
    b.record(ElementKind::Fact, "Alpha", Verification::Verified);
    b.record(ElementKind::Assert, "Zeta", Verification::Verified);

    assert_eq!(a.render(), b.render());
}

/// Facts come before asserts, and each group is sorted by name.
#[test]
fn rendering_groups_facts_before_asserts_and_sorts_by_name() {
    let mut c = Coverage::new();
    c.record(ElementKind::Assert, "B", Verification::Verified);
    c.record(ElementKind::Fact, "Z", Verification::Verified);
    c.record(ElementKind::Assert, "A", Verification::Verified);
    c.record(ElementKind::Fact, "Y", Verification::Verified);

    let rendered = c.render();
    let names: Vec<&str> = rendered.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split_whitespace().nth(2).unwrap())
        .collect();
    assert_eq!(names, vec!["Y", "Z", "A", "B"]);
}

/// A fact is verified if *any* emitter produced a real assertion for it. The
/// cross-test and pairwise scaffolds are derived artifacts, so a `Declined`
/// from one of them must not mask the invariant test that does the work.
#[test]
fn a_stronger_status_wins_however_the_records_arrive() {
    for (first, second) in [
        (Verification::Declined("scaffold".into()), Verification::Verified),
        (Verification::Verified, Verification::Declined("scaffold".into())),
    ] {
        let mut c = Coverage::new();
        c.record(ElementKind::Fact, "F", first);
        c.record(ElementKind::Fact, "F", second);
        let lines = entry_lines(&c);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("verified"), "verified must win: {lines:?}");
    }
}

/// `by-type` outranks a decline — a constraint the type system encodes needs no
/// test, so an emitter that skipped it is not a gap — but loses to a real
/// assertion.
#[test]
fn by_type_outranks_declined_and_loses_to_verified() {
    let mut c = Coverage::new();
    c.record(ElementKind::Fact, "F", Verification::Declined("no".into()));
    c.record(ElementKind::Fact, "F", Verification::ByType);
    assert!(entry_lines(&c)[0].starts_with("by-type"), "{:?}", entry_lines(&c));

    c.record(ElementKind::Fact, "F", Verification::Verified);
    assert!(entry_lines(&c)[0].starts_with("verified"), "{:?}", entry_lines(&c));
}

/// One element is one line: recording the same status twice must not duplicate.
#[test]
fn an_element_appears_exactly_once() {
    let mut c = Coverage::new();
    c.record(ElementKind::Fact, "F", Verification::Verified);
    c.record(ElementKind::Fact, "F", Verification::Verified);
    let lines = c.render().lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .count();
    assert_eq!(lines, 1, "{}", c.render());
}

/// `check` reads back what `generate` wrote, so the pair must round-trip —
/// including a decline's reason, which is the only place the gap is explained.
#[test]
fn a_rendered_manifest_parses_back_to_the_same_entries() {
    let mut c = Coverage::new();
    c.record(ElementKind::Fact, "Paired",
        Verification::Declined("a transition over 2 bindings has no pre/post pairing".into()));
    c.record(ElementKind::Fact, "NoSelfRef", Verification::Verified);
    c.record(ElementKind::Assert, "Refl", Verification::ByType);

    let parsed = Coverage::parse(&c.render()).expect("round-trip");
    assert_eq!(parsed.render(), c.render());

    let declined: Vec<_> = parsed.declined().collect();
    assert_eq!(declined.len(), 1);
    assert_eq!(declined[0].name, "Paired");
    assert!(declined[0].reason().unwrap().contains("pre/post pairing"));
}

/// A reason is free prose written by an emitter. It must not be able to end the
/// line early and forge a second entry.
#[test]
fn a_reason_containing_a_newline_cannot_forge_an_entry() {
    let mut c = Coverage::new();
    c.record(ElementKind::Fact, "F",
        Verification::Declined("first\nverified fact Forged".into()));
    let rendered = c.render();
    assert!(!rendered.contains("Forged\n") || Coverage::parse(&rendered).unwrap().declined().count() == 1);

    let parsed = Coverage::parse(&rendered).expect("parse");
    let names: Vec<String> = parsed.entries().map(|e| e.name.clone()).collect();
    assert_eq!(names, vec!["F"], "a newline in a reason forged an entry:\n{rendered}");
}

/// An unreadable manifest is a bug in oxidtr, not a silent pass.
#[test]
fn a_malformed_line_is_an_error_not_a_skip() {
    assert!(Coverage::parse("nonsense\n").is_err());
    assert!(Coverage::parse("verified fact\n").is_err());
    assert!(Coverage::parse("bogus fact F\n").is_err());
}

/// Comments and blank lines are ignored so the manifest can carry a header.
#[test]
fn comments_and_blank_lines_are_ignored() {
    let c = Coverage::parse("# oxidtr coverage\n\nverified fact F\n").expect("parse");
    assert_eq!(c.entries().count(), 1);
}

// ── backend wiring ─────────────────────────────────────────────────────────

fn manifest_of(files: &[oxidtr::backend::GeneratedFile]) -> Coverage {
    let raw = files.iter().find(|f| f.path == "coverage.txt")
        .unwrap_or_else(|| panic!("no coverage.txt in {:?}",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()));
    Coverage::parse(&raw.content).expect("the manifest a backend wrote must parse")
}

fn kotlin(model: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let m = oxidtr::parser::parse(model).expect("parse");
    let ir = oxidtr::ir::lower(&m).expect("lower");
    oxidtr::backend::jvm::kotlin::generate(&ir)
}

/// The shape that started this: the whole test body is the note explaining that
/// the guarantee was dropped, and the old substring check read that note as the
/// proof.
#[test]
fn kotlin_records_a_multi_binding_transition_as_declined() {
    let files = kotlin("sig Foo { var tag: one Int }\n\
                        fact Paired { always all a, b: Foo | a.tag' = b.tag }");
    let declined: Vec<_> = manifest_of(&files).declined().collect();
    assert_eq!(declined.len(), 1, "{declined:?}");
    assert_eq!(declined[0].name, "Paired");
    assert!(declined[0].reason().unwrap().contains("pre/post pairing"),
        "the reason must say what could not be expressed: {:?}", declined[0].reason());
}

/// A fact whose domain is populated really is asserted, so it is not a gap.
#[test]
fn kotlin_records_an_asserted_fact_as_verified() {
    let files = kotlin("sig P { x: one Int }\nfact CardOne { all p: P | p.x = 0 }");
    let m = manifest_of(&files);
    assert_eq!(m.declined().count(), 0, "{}", m.render());
    assert!(m.entries().any(|e| e.name == "CardOne"
        && e.status == Verification::Verified), "{}", m.render());
}

/// `Knot` has no finite value, so its domain cannot be populated. The old
/// output asserted over an empty list and went green; it must now be declined
/// *and* not run.
#[test]
fn kotlin_declines_a_vacuous_domain_and_does_not_assert_it() {
    let files = kotlin("sig Knot { other: one Knot }\n\
                        fact Trivial { all k: Knot | k = k }");
    let declined: Vec<_> = manifest_of(&files).declined().collect();
    assert_eq!(declined.len(), 1, "{declined:?}");
    assert_eq!(declined[0].name, "Trivial");

    let tests = files.iter().find(|f| f.path == "Tests.kt").expect("Tests.kt").content.clone();
    let body = tests.split("invariant Trivial").nth(1).unwrap_or(&tests);
    assert!(tests.contains("@Disabled(\"oxidtr:"),
        "a vacuous test must not run — the runner has to say so too:\n{tests}");
    assert!(body.contains("assertTrue"),
        "the assertion stays for the reader; @Disabled is what stops it:\n{tests}");
}

/// An element the type system covers is not a gap. Without this the manifest
/// would make the strongest targets look like the worst.
#[test]
fn kotlin_records_a_type_guaranteed_fact_as_by_type() {
    let files = kotlin("sig P { x: lone Int }\nfact Present { all p: P | some p.x }");
    let m = manifest_of(&files);
    let by_type: Vec<_> = m.entries().filter(|e| e.status == Verification::ByType).collect();
    assert!(!by_type.is_empty(), "expected a by-type element:\n{}", m.render());
    assert_eq!(m.declined().count(), 0, "{}", m.render());
}

/// The manifest exists even with nothing to say, so `check` can tell a
/// generated implementation from a hand-written one.
#[test]
fn kotlin_writes_a_manifest_even_with_no_facts() {
    let files = kotlin("sig Lonely {}");
    assert_eq!(manifest_of(&files).entries().count(), 0);
}
