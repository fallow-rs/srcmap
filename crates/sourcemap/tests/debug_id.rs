//! Tests for the `debugId` comment of the ECMA-426 debug ID proposal.

use srcmap_sourcemap::{extract_css_debug_id, extract_debug_id};

const ID: &str = "85314830-023f-4cf1-a267-535f4e37bb17";

#[test]
fn reads_the_javascript_comment_on_the_last_line() {
    let source = format!("console.log(1);\n//# debugId={ID}");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn reads_the_comment_before_a_final_line_end() {
    let source = format!("console.log(1);\n//# debugId={ID}\n");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn reads_the_css_comment() {
    let source = format!("body{{color:red}}\n/*# debugId={ID} */\n");
    assert_eq!(extract_css_debug_id(&source).as_deref(), Some(ID));

    let no_space = format!("body{{color:red}}\n/*# debugId={ID}*/");
    assert_eq!(extract_css_debug_id(&no_space).as_deref(), Some(ID));

    let tab = format!("body{{color:red}}\n/*# debugId={ID}\t */");
    assert_eq!(extract_css_debug_id(&tab).as_deref(), Some(ID));
}

#[test]
fn each_comment_form_belongs_to_its_file_type() {
    let css_form = format!("var a;\n/*# debugId={ID} */\n");
    assert_eq!(extract_debug_id(&css_form), None);

    let js_form = format!("body{{color:red}}\n//# debugId={ID}\n");
    assert_eq!(extract_css_debug_id(&js_form), None);
}

#[test]
fn each_javascript_line_terminator_ends_a_line() {
    for terminator in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
        let after = format!("var a;{terminator}//# debugId={ID}");
        assert_eq!(extract_debug_id(&after).as_deref(), Some(ID), "after {terminator:?}");

        let before = format!("var a;\n//# debugId={ID}{terminator}");
        assert_eq!(extract_debug_id(&before).as_deref(), Some(ID), "before {terminator:?}");

        let css = format!("a{{}}{terminator}/*# debugId={ID} */{terminator}");
        assert_eq!(extract_css_debug_id(&css).as_deref(), Some(ID), "CSS {terminator:?}");
    }
}

#[test]
fn each_javascript_line_terminator_counts_as_one_line() {
    for terminator in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
        let fifth = format!("//# debugId={ID}{t}1;{t}2;{t}3;{t}4;{t}", t = terminator);
        assert_eq!(extract_debug_id(&fifth).as_deref(), Some(ID), "fifth with {terminator:?}");

        let sixth = format!("//# debugId={ID}{t}1;{t}2;{t}3;{t}4;{t}5;{t}", t = terminator);
        assert_eq!(extract_debug_id(&sixth), None, "sixth with {terminator:?}");
    }
}

#[test]
fn reads_the_comment_above_a_source_mapping_url_comment() {
    let source = format!("var a;\n//# debugId={ID}\n//# sourceMappingURL=app.js.map\n");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn reads_the_comment_below_a_source_mapping_url_comment() {
    let source = format!("var a;\n//# sourceMappingURL=app.js.map\n//# debugId={ID}\n");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn returns_lower_case_for_upper_case_hex() {
    let source = format!("var a;\n//# debugId={}\n", ID.to_ascii_uppercase());
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn reads_crlf_line_ends() {
    let source = format!("var a;\r\n//# debugId={ID}\r\n//# sourceMappingURL=app.js.map\r\n");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn returns_none_without_a_comment() {
    assert_eq!(extract_debug_id(""), None);
    assert_eq!(extract_debug_id("var a;\n//# sourceMappingURL=app.js.map\n"), None);
}

#[test]
fn reads_the_fifth_line_from_the_end() {
    let source = format!("var a;\n//# debugId={ID}\n1;\n2;\n3;\n4;\n");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));
}

#[test]
fn ignores_the_sixth_line_from_the_end() {
    let source = format!("//# debugId={ID}\n1;\n2;\n3;\n4;\n5;\n");
    assert_eq!(extract_debug_id(&source), None);
}

#[test]
fn counts_empty_lines_at_the_end() {
    let source = format!("//# debugId={ID}\n\n\n\n\n\n");
    assert_eq!(extract_debug_id(&source), None);
}

#[test]
fn ignores_an_invalid_uuid() {
    for id in [
        "85314830023f4cf1a267535f4e37bb17",
        "85314830-023f-4cf1-a267-535f4e37bb1",
        "85314830-023f-4cf1-a267-535f4e37bb170",
        "8531483g-023f-4cf1-a267-535f4e37bb17",
        "85314830_023f_4cf1_a267_535f4e37bb17",
        "{85314830-023f-4cf1-a267-535f4e37bb17}",
        "",
    ] {
        let source = format!("var a;\n//# debugId={id}\n");
        assert_eq!(extract_debug_id(&source), None, "accepted {id:?}");
    }
}

#[test]
fn ignores_a_comment_that_does_not_start_the_line() {
    let after_code = format!("var a; //# debugId={ID}\n");
    assert_eq!(extract_debug_id(&after_code), None);

    let indented = format!("var a;\n  //# debugId={ID}\n");
    assert_eq!(extract_debug_id(&indented), None);
}

#[test]
fn ignores_text_after_the_uuid() {
    let source = format!("var a;\n//# debugId={ID} extra\n");
    assert_eq!(extract_debug_id(&source), None);
}

#[test]
fn ignores_other_comment_markers() {
    let legacy = format!("var a;\n//@ debugId={ID}\n");
    assert_eq!(extract_debug_id(&legacy), None);

    let wrong_case = format!("var a;\n//# debugid={ID}\n");
    assert_eq!(extract_debug_id(&wrong_case), None);
}

#[test]
fn uses_the_last_valid_comment() {
    let other = "00000000-0000-4000-8000-000000000000";
    let source = format!("//# debugId={other}\n//# debugId={ID}\n");
    assert_eq!(extract_debug_id(&source).as_deref(), Some(ID));

    let invalid_last = format!("//# debugId={ID}\n//# debugId=not-a-uuid\n");
    assert_eq!(extract_debug_id(&invalid_last).as_deref(), Some(ID));
}
