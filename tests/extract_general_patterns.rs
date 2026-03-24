/// Mine tests for general patterns found in hand-written (non-oxidtr-generated) code.
/// Organized by pattern category following TDD: tests written first, then implementation.

use oxidtr::extract::{rust_extractor, ts_extractor, kotlin_extractor, java_extractor, Confidence};

// ── Category 1: Validation patterns ──────────────────────────────────────────

#[test]
fn rust_assert_greater_than() {
    let src = "pub fn validate(x: i32) {\n    assert!(x > 0);\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High && f.alloy_text.contains("> 0")
        ),
        "assert!(x > 0) should produce High confidence fact with '> 0': {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_assert_len_constraint() {
    let src = "pub fn validate(items: &[i32]) {\n    assert!(items.len() <= 10);\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High
            && (f.alloy_text.contains("#items <= 10") || f.alloy_text.contains("<= 10"))
        ),
        "assert!(items.len() <= 10) should produce fact with cardinality constraint: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_debug_assert_not_empty() {
    let src = "pub fn process(items: &[i32]) {\n    debug_assert!(!items.is_empty());\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High && f.alloy_text.contains("items")
        ),
        "debug_assert!(!items.is_empty()) should produce High confidence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_require_gte() {
    let src = "fun validate(age: Int) {\n    require(age >= 0)\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High && f.alloy_text.contains(">= 0")
        ),
        "require(age >= 0) should produce High confidence fact with '>= 0': {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_require_not_empty() {
    let src = "fun validate(name: String) {\n    require(name.isNotEmpty())\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High && f.alloy_text.contains("name")
        ),
        "require(name.isNotEmpty()) should produce High confidence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_check_size_constraint() {
    let src = "fun validate(items: List<String>, maxSize: Int) {\n    check(items.size <= maxSize)\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High
            && (f.alloy_text.contains("#items <= maxSize") || f.alloy_text.contains("<= maxSize"))
        ),
        "check(items.size <= maxSize) should produce High confidence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_objects_require_non_null() {
    let src = "public static void init(String name) {\n    Objects.requireNonNull(name);\n}\n";
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::High
            && f.alloy_text.contains("name")
            && f.source_pattern.contains("requireNonNull")
        ),
        "Objects.requireNonNull(name) should produce High confidence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_assert_size_constraint() {
    let src = "public static void validate(List<String> items) {\n    assert items.size() <= 10;\n}\n";
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Medium
            && (f.alloy_text.contains("#items <= 10") || f.alloy_text.contains("<= 10"))
        ),
        "assert items.size() <= 10 should produce Medium fact with size constraint: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_throw_illegal_argument() {
    let src = "public static void validate(int x) {\n    if (x < 0) throw new IllegalArgumentException();\n}\n";
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Low
            && f.alloy_text.contains("x >= 0")
        ),
        "if (x < 0) throw should produce Low fact with negated condition: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_null_throw_presence() {
    let src = "function init(x: string | null) {\n    if (x === null) throw new Error('required');\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Medium
            && f.alloy_text.contains("x")
            && f.source_pattern.contains("null guard")
        ),
        "if (x === null) throw should produce Medium fact about presence: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_array_type_guard() {
    let src = "function process(items: unknown) {\n    if (!Array.isArray(items)) throw new Error('expected array');\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Low
            && f.alloy_text.contains("items")
            && f.source_pattern.contains("type guard")
        ),
        "Array.isArray guard should produce Low fact: {:?}",
        mined.fact_candidates
    );
}

// ── Category 2: Guard clause patterns ────────────────────────────────────────

#[test]
fn rust_guard_empty_return_err() {
    let src = "pub fn process(items: &[i32]) -> Result<(), String> {\n    if items.is_empty() { return Err(\"no items\".into()); }\n    Ok(())\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("some items") || f.alloy_text.contains("items")
        ),
        "guard on is_empty should produce fact about items: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_guard_none_return_err() {
    let src = "pub fn process(x: Option<i32>) -> Result<i32, String> {\n    if x.is_none() { return Err(\"missing\".into()); }\n    Ok(x.unwrap())\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("x") && f.source_pattern.contains("is_none guard")
        ),
        "guard on is_none should produce presence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_let_some_else_return() {
    let src = "pub fn process(field: Option<i32>) -> Result<i32, String> {\n    let Some(x) = field else { return Err(\"missing\".into()); };\n    Ok(x)\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field") && f.source_pattern.contains("let-else")
        ),
        "let Some(x) = field else should produce presence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_elvis_throw() {
    let src = "fun process(name: String?) {\n    val n = name ?: throw IllegalStateException(\"required\")\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("name")
            && f.source_pattern.contains("elvis throw")
        ),
        "?: throw should produce presence constraint fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_safe_call_elvis_return() {
    let src = "fun process(name: String?) {\n    val len = name?.length ?: return\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("name")
            && f.source_pattern.contains("elvis")
        ),
        "?.let ?: return should produce lone field fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_null_check_throw() {
    let src = "public static void init(String field) {\n    if (field == null) throw new NullPointerException();\n}\n";
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field")
            && f.source_pattern.contains("null guard")
        ),
        "if (field == null) throw should produce presence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_optional_or_else_throw() {
    let src = "public static String getValue(Optional<String> opt) {\n    return opt.orElseThrow();\n}\n";
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("opt")
            && f.source_pattern.contains("orElseThrow")
        ),
        "Optional.orElseThrow() should produce presence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_null_undefined_throw() {
    let src = "function init(x: string | null | undefined) {\n    if (x === null || x === undefined) throw new Error('missing');\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Medium
            && f.alloy_text.contains("x")
            && f.source_pattern.contains("null guard")
        ),
        "null || undefined throw should produce presence fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_non_null_assertion() {
    let src = "function process(x: string | null) {\n    console.log(x!.length);\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Low
            && f.alloy_text.contains("x")
            && f.source_pattern.contains("non-null assertion")
        ),
        "x! should produce Low presence hint: {:?}",
        mined.fact_candidates
    );
}

// ── Category 3: Match/switch exhaustiveness ──────────────────────────────────

#[test]
fn rust_match_enum_variants() {
    let src = r#"
pub fn display(status: Status) -> &str {
    match status {
        Status::Active => "active",
        Status::Inactive => "inactive",
        Status::Pending => "pending",
    }
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("match exhaustiveness")
            && f.alloy_text.contains("Active")
            && f.alloy_text.contains("Inactive")
            && f.alloy_text.contains("Pending")
        ),
        "match arms should produce enum variants evidence: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_when_sealed() {
    let src = r#"
fun display(status: Status): String {
    return when (status) {
        is Active -> "active"
        is Inactive -> "inactive"
    }
}
"#;
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("when exhaustiveness")
            && f.alloy_text.contains("Active")
            && f.alloy_text.contains("Inactive")
        ),
        "when arms should produce variant evidence: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_switch_enum() {
    let src = r#"
public static String display(Status status) {
    switch (status) {
        case Active: return "active";
        case Inactive: return "inactive";
    }
}
"#;
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("switch exhaustiveness")
            && f.alloy_text.contains("Active")
            && f.alloy_text.contains("Inactive")
        ),
        "switch cases should produce variant evidence: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_switch_pattern_matching() {
    let src = r#"
public static String display(Shape shape) {
    return switch (shape) {
        case Circle c -> "circle";
        case Square s -> "square";
    };
}
"#;
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("switch exhaustiveness")
            && f.alloy_text.contains("Circle")
            && f.alloy_text.contains("Square")
        ),
        "Java 21 pattern switch should produce variant evidence: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_switch_discriminated_union() {
    let src = r#"
function display(status: Status) {
    switch (status.kind) {
        case "Active": return "active";
        case "Inactive": return "inactive";
        case "Pending": return "pending";
    }
}
"#;
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("switch exhaustiveness")
            && f.alloy_text.contains("Active")
            && f.alloy_text.contains("Inactive")
            && f.alloy_text.contains("Pending")
        ),
        "switch on discriminant should produce variant evidence: {:?}",
        mined.fact_candidates
    );
}

// ── Category 4: Null/Option handling patterns ────────────────────────────────

#[test]
fn rust_if_let_some() {
    let src = "pub fn process(field: Option<i32>) {\n    if let Some(x) = field {\n        println!(\"{}\", x);\n    }\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field")
            && f.source_pattern.contains("if let Some")
        ),
        "if let Some(x) = field should produce lone fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_unwrap_hint() {
    let src = "pub fn process(field: Option<i32>) {\n    let x = field.unwrap();\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Low
            && f.alloy_text.contains("field")
            && f.source_pattern.contains("unwrap")
        ),
        "field.unwrap() should produce Low presence hint: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_unwrap_or_default() {
    let src = "pub fn process(field: Option<i32>) {\n    let x = field.unwrap_or(0);\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field")
            && f.source_pattern.contains("unwrap_or")
        ),
        "field.unwrap_or(default) should produce lone-with-default fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_safe_call() {
    let src = "fun process(field: String?) {\n    val len = field?.length\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field")
            && f.source_pattern.contains("safe call")
        ),
        "field?.method() should produce lone fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_double_bang() {
    let src = "fun process(field: String?) {\n    val len = field!!.length\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.confidence == Confidence::Low
            && f.alloy_text.contains("field")
            && f.source_pattern.contains("non-null assertion")
        ),
        "field!! should produce Low presence hint: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_optional_chaining() {
    let src = "function process(field: User | null) {\n    const name = field?.name;\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field")
            && f.source_pattern.contains("optional chaining")
        ),
        "field?.name should produce lone fact: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_nullish_coalescing() {
    let src = "function process(field: string | null) {\n    const val = field ?? 'default';\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.alloy_text.contains("field")
            && f.source_pattern.contains("nullish coalescing")
        ),
        "field ?? default should produce lone-with-default fact: {:?}",
        mined.fact_candidates
    );
}

// ── Category 5: Collection operation patterns ────────────────────────────────

#[test]
fn rust_filter_chain() {
    let src = "pub fn active_users(users: &[User]) -> Vec<&User> {\n    users.iter().filter(|u| u.active).collect()\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("filter")
            && f.alloy_text.contains("users")
        ),
        ".filter() should produce subset relation candidate: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_filter_chain() {
    let src = "function activeUsers(users: User[]) {\n    return users.filter(u => u.active);\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("filter")
            && f.alloy_text.contains("users")
        ),
        ".filter() should produce subset relation candidate: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn kotlin_filter_chain() {
    let src = "fun activeUsers(users: List<User>): List<User> {\n    return users.filter { it.active }\n}\n";
    let mined = kotlin_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("filter")
            && f.alloy_text.contains("users")
        ),
        ".filter should produce subset relation candidate: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn java_stream_filter() {
    let src = "public static List<User> activeUsers(List<User> users) {\n    return users.stream().filter(u -> u.active()).collect(Collectors.toList());\n}\n";
    let mined = java_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("filter")
            && f.alloy_text.contains("users")
        ),
        ".stream().filter() should produce subset relation candidate: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn rust_map_chain() {
    let src = "pub fn names(users: &[User]) -> Vec<String> {\n    users.iter().map(|u| u.name.clone()).collect()\n}\n";
    let mined = rust_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("map")
            && f.alloy_text.contains("users")
        ),
        ".map() should produce field mapping candidate: {:?}",
        mined.fact_candidates
    );
}

#[test]
fn ts_map_chain() {
    let src = "function names(users: User[]) {\n    return users.map(u => u.name);\n}\n";
    let mined = ts_extractor::extract(src);
    assert!(
        mined.fact_candidates.iter().any(|f|
            f.source_pattern.contains("map")
            && f.alloy_text.contains("users")
        ),
        ".map() should produce field mapping candidate: {:?}",
        mined.fact_candidates
    );
}
