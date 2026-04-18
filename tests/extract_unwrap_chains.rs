//! Regression tests for fact-candidate extraction on chain-call patterns.
//!
//! The `.unwrap()` / `.unwrap_or(...)` extractors used to produce malformed
//! Alloy text like `fact { ) is one (unsafe unwrap) }` when the receiver
//! was a method-chain rather than a bare identifier — `rsplit` on
//! delimiters `' '`, `'='`, `'('` would return the trailing `)` of the
//! chain rather than the last identifier.
//!
//! The extractor must now produce either a valid identifier chain or
//! nothing — never a lone closing paren.

use oxidtr::extract::rust_extractor;

fn fact_texts(src: &str) -> Vec<String> {
    rust_extractor::extract(src)
        .fact_candidates
        .into_iter()
        .map(|f| f.alloy_text)
        .collect()
}

fn has_broken_candidate(texts: &[String]) -> bool {
    // Any candidate whose "field" portion starts with ')' is malformed.
    // Every well-formed candidate in this extractor starts with an
    // identifier character (letter or underscore).
    texts.iter().any(|t| {
        let trimmed = t.trim_start();
        trimmed.starts_with(')')
    })
}

#[test]
fn unwrap_on_method_chain_does_not_produce_dangling_paren() {
    // Reproduces the Asterinas `rcu.read().get().unwrap()` pattern.
    let src = r#"
pub fn f(rcu: &Rcu) {
    let _ = rcu.read().get().unwrap();
}
"#;
    let texts = fact_texts(src);
    assert!(
        !has_broken_candidate(&texts),
        "extracted a dangling ) as an identifier. Got:\n{texts:#?}"
    );
}

#[test]
fn double_paren_chain_unwrap_does_not_produce_dangling_paren() {
    // Reproduces the `foo((x)).unwrap()` pattern that previously
    // yielded `fact { )) is one (unsafe unwrap) }`.
    let src = r#"
pub fn f(x: Option<Option<u32>>) {
    let _ = x.transpose().unwrap();
    let _ = Some(Some(1u32)).flatten().unwrap();
}
"#;
    let texts = fact_texts(src);
    assert!(
        !has_broken_candidate(&texts),
        "extracted a dangling )) as an identifier. Got:\n{texts:#?}"
    );
}

#[test]
fn unwrap_or_on_method_chain_does_not_produce_dangling_paren() {
    let src = r#"
pub fn f(m: &Map) {
    let _ = m.get("k").unwrap_or(&0);
    let _ = parse(input).unwrap_or_default();
}
"#;
    let texts = fact_texts(src);
    assert!(
        !has_broken_candidate(&texts),
        "unwrap_or variants also must not leak ). Got:\n{texts:#?}"
    );
}

#[test]
fn bare_identifier_unwrap_still_produces_candidate() {
    // Sanity: the legitimate case must still produce a candidate.
    let src = r#"
pub fn f(maybe_name: Option<String>) {
    let name = maybe_name.unwrap();
    println!("{}", name);
}
"#;
    let texts = fact_texts(src);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("maybe_name") && t.contains("unsafe unwrap")),
        "should still extract `maybe_name is one (unsafe unwrap)`. Got:\n{texts:#?}"
    );
}
