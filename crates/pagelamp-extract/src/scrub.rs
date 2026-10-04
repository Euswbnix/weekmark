//! Access parameters in link addresses.
//!
//! A link address in LMS HTML can carry a parameter that opens the file for whoever has the
//! address (`verifier`, `sf_verifier`, `access_token`). Such a parameter must never reach
//! stored text, the search index or anything PageLamp gives out. [`scrub_text`] removes them
//! from every address in a text:
//!
//! - an address to a file (`…/files/<id>`, also with `/download` or `/preview`) loses its
//!   whole query;
//! - any other address loses just those parameters, in its query and in its fragment, and any
//!   parameter whose own value carries one (a link wrapped by a mail gateway);
//! - a `name=value` of one of those parameters is removed also when the address is cut before
//!   it: text extracted from a PDF can break a line after the "?", and a search snippet can
//!   start at the name.
//!
//! It is applied to all text before it is stored, and again where text leaves PageLamp, which
//! covers text stored by an earlier version. Applying it to its own result changes nothing
//! more. One linear pass, no regular expressions.
//!
//! Where an address ends. In running text nothing marks the end of an address, so one ends at
//! whitespace, a quote or a bracket (extracted HTML writes a link as `text (address)`), a
//! backslash (JSON output escapes the quote or line break after an address with one) and at
//! the first character that isn't ASCII: Chinese text follows an address without a space, and
//! a real address is written in ASCII. A full stop, comma or the like at the very end belongs
//! to the sentence and stays.
//!
//! Not recognised, deliberately: a parameter name written percent-encoded or with invisible
//! characters inside it, and a value that stands alone (on the next line of a PDF, or at the
//! very start of a search snippet). For the last, callers that give out a snippet of stored
//! text check the whole text it was cut from (`pagelamp-mcp`).

use std::borrow::Cow;

/// The rules' version. PageLamp records which version its stored text was last cleaned under
/// and cleans it again when this is raised; raise it when the rules change.
pub const VERSION: u32 = 1;

/// Parameters that give access by themselves.
const ACCESS_PARAMETERS: [&str; 3] = ["verifier", "sf_verifier", "access_token"];

/// `text` without access parameters in the addresses it contains. Borrowed when there is
/// nothing to remove.
pub fn scrub_text(text: &str) -> Cow<'_, str> {
    if !may_need_scrubbing(text) {
        return Cow::Borrowed(text);
    }
    let mut out = String::new();
    let mut copied = 0; // `text[..copied]` is already in `out`
    let mut word_start = None;
    let scrub_word = |start: usize, end: usize, out: &mut String, copied: &mut usize| {
        // Punctuation at the very end belongs to the sentence around the address.
        let core = text[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if let Some(clean) = scrub_address(core) {
            out.push_str(&text[*copied..start]);
            out.push_str(&clean);
            *copied = start + core.len();
        }
    };
    for (at, c) in text.char_indices() {
        if ends_an_address(c) {
            if let Some(start) = word_start.take() {
                scrub_word(start, at, &mut out, &mut copied);
            }
        } else if word_start.is_none() {
            word_start = Some(at);
        }
    }
    if let Some(start) = word_start {
        scrub_word(start, text.len(), &mut out, &mut copied);
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// A cheap test that lets almost every text through untouched. It looks through the search
/// marks, which can sit inside a parameter's name.
fn may_need_scrubbing(text: &str) -> bool {
    let unmarked: Cow<'_, str> = if text.contains(is_search_mark) {
        Cow::Owned(text.chars().filter(|c| !is_search_mark(*c)).collect())
    } else {
        Cow::Borrowed(text)
    };
    // ("verifier" is also in "sf_verifier".)
    contains_ignoring_case(&unmarked, "verifier")
        || contains_ignoring_case(&unmarked, "access_token")
        || (unmarked.contains('?') && contains_ignoring_case(&unmarked, "/files/"))
}

fn contains_ignoring_case(text: &str, needle: &str) -> bool {
    let (text, needle) = (text.as_bytes(), needle.as_bytes());
    text.windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Whether an address can't go on with `c` (see the module docs).
fn ends_an_address(c: char) -> bool {
    if is_search_mark(c) {
        return false;
    }
    !c.is_ascii()
        || c.is_ascii_whitespace()
        || c.is_ascii_control()
        || matches!(
            c,
            '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '`' | '\\'
        )
}

/// The marks the search index puts around a match in a snippet. They can sit inside an
/// address (`?«verifier»=…`), so they don't end one, and names are compared without them.
fn is_search_mark(c: char) -> bool {
    matches!(c, '«' | '»')
}

/// `word` without its access parameters. `None`: nothing to change.
fn scrub_address(word: &str) -> Option<String> {
    // The fragment first: it can carry parameters too, or an address with a query of its own.
    let (main, fragment) = match word.split_once('#') {
        Some((main, fragment)) => (main, Some(fragment)),
        None => (word, None),
    };
    let clean_main = scrub_before_fragment(main);
    let clean_fragment = fragment.and_then(scrub_address);
    if clean_main.is_none() && clean_fragment.is_none() {
        return None;
    }
    let mut clean = clean_main.unwrap_or_else(|| main.to_string());
    match clean_fragment.as_deref().or(fragment) {
        Some(fragment) if !fragment.is_empty() => {
            clean.push('#');
            clean.push_str(fragment);
        }
        _ => {}
    }
    Some(clean)
}

/// An address up to its fragment: `…?query`, or what is left of one that was cut before here.
fn scrub_before_fragment(main: &str) -> Option<String> {
    let Some((before, query)) = main.split_once('?') else {
        // No "?": the address was cut before this word, or there never was one. A `name=value`
        // of an access parameter goes all the same (a bare name is an ordinary word).
        return without_access_pairs(main, false);
    };
    if before.is_empty() {
        // A word that starts with "?" isn't an address (code such as `"?access_token=" + t`).
        return None;
    }
    let query = if is_file_address(before) {
        (!query.is_empty()).then(String::new)?
    } else {
        without_access_pairs(query, true)?
    };
    Some(if query.is_empty() {
        before.to_string()
    } else {
        format!("{before}?{query}")
    })
}

/// `pairs` (a query, a fragment, or a word an address was cut before) without the pairs that
/// name an access parameter or carry one in their value. Pairs are separated by `&`, `;` or
/// an undecoded `&amp;`; the first pair that stays loses its separator. `bare_names`: a name
/// without a value counts too (inside a query). `None`: none removed.
fn without_access_pairs(pairs: &str, bare_names: bool) -> Option<String> {
    let mut kept = String::new();
    let mut removed = false;
    let mut rest = pairs;
    let mut separator = "";
    loop {
        let end = rest.find(['&', ';']).unwrap_or(rest.len());
        let pair = &rest[..end];
        if is_access_pair(pair, bare_names) {
            removed = true;
        } else {
            if !kept.is_empty() {
                kept.push_str(separator);
            }
            kept.push_str(pair);
        }
        if end == rest.len() {
            break;
        }
        let after = &rest[end..];
        separator = if after.starts_with("&amp;") {
            "&amp;"
        } else {
            &after[..1]
        };
        rest = &after[separator.len()..];
    }
    removed.then_some(kept)
}

/// `name=value` naming an access parameter, in any letter case and with or without search
/// marks (with `bare_names`, also the name alone); or any pair whose value carries one, plainly
/// or with its `=` encoded.
fn is_access_pair(pair: &str, bare_names: bool) -> bool {
    let (name, value) = match pair.split_once('=') {
        Some(split) => split,
        None if bare_names => (pair, ""),
        None => return false,
    };
    let name: String = name.chars().filter(|c| !is_search_mark(*c)).collect();
    if ACCESS_PARAMETERS
        .iter()
        .any(|access| name.eq_ignore_ascii_case(access))
    {
        return true;
    }
    [
        "verifier=",
        "verifier%3d",
        "access_token=",
        "access_token%3d",
    ]
    .iter()
    .any(|carried| contains_ignoring_case(value, carried))
}

/// Whether the part of an address before its query ends in `/files/<id>`, `/files/<id>/download`
/// or `/files/<id>/preview` (an id is digits, or digits with `~` for another shard).
fn is_file_address(before_query: &str) -> bool {
    let lower = before_query
        .chars()
        .filter(|c| !is_search_mark(*c))
        .collect::<String>()
        .to_ascii_lowercase();
    let Some(at) = lower.rfind("/files/") else {
        return false;
    };
    let after = &lower[at + "/files/".len()..];
    let id_len = after
        .bytes()
        .take_while(|b| b.is_ascii_digit() || *b == b'~')
        .count();
    id_len > 0 && matches!(&after[id_len..], "" | "/" | "/download" | "/preview")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_address_loses_its_whole_query() {
        for (address, clean) in [
            (
                "https://lms.example.edu/courses/101/files/501/download?verifier=DEMOSECRET&wrap=1",
                "https://lms.example.edu/courses/101/files/501/download",
            ),
            (
                "https://lms.example.edu/files/501/preview?verifier=DEMOSECRET",
                "https://lms.example.edu/files/501/preview",
            ),
            (
                "/courses/101/files/1234~501?wrap=1",
                "/courses/101/files/1234~501",
            ),
            (
                "https://lms.example.edu/users/7/files/501/download?download_frd=1&sf_verifier=DEMOSECRET#page=2",
                "https://lms.example.edu/users/7/files/501/download#page=2",
            ),
        ] {
            assert_eq!(scrub_text(address), clean, "{address}");
            let in_text = format!("Week 3 slides ({address}), due Friday.");
            assert_eq!(
                scrub_text(&in_text),
                format!("Week 3 slides ({clean}), due Friday.")
            );
        }
    }

    #[test]
    fn other_addresses_lose_only_the_access_parameters() {
        for (address, clean) in [
            (
                "https://lms.example.edu/courses/101/pages/week-3?module_item_id=9&verifier=DEMOSECRET",
                "https://lms.example.edu/courses/101/pages/week-3?module_item_id=9",
            ),
            (
                "https://media.example.edu/watch?access_token=DEMOSECRET&t=30#intro",
                "https://media.example.edu/watch?t=30#intro",
            ),
            (
                "https://media.example.edu/watch?access_token=DEMOSECRET",
                "https://media.example.edu/watch",
            ),
            // Any letter case, an undecoded `&amp;`, `;` between pairs, a name with no value.
            (
                "https://media.example.edu/watch?a=1&amp;Verifier=DEMOSECRET&amp;b=2&SF_VERIFIER",
                "https://media.example.edu/watch?a=1&amp;b=2",
            ),
            (
                "https://media.example.edu/watch?access_token=DEMOSECRET&amp;t=30",
                "https://media.example.edu/watch?t=30",
            ),
            (
                "https://media.example.edu/watch?a=1;verifier=DEMOSECRET;b=2",
                "https://media.example.edu/watch?a=1;b=2",
            ),
            // In the fragment, as pairs or as an address with a query of its own.
            (
                "https://app.example.edu/#access_token=DEMOSECRET&state=7",
                "https://app.example.edu/#state=7",
            ),
            (
                "https://app.example.edu/view?x=1#/doc?verifier=DEMOSECRET&p=2",
                "https://app.example.edu/view?x=1#/doc?p=2",
            ),
            // A link wrapped by a gateway carries the whole address as a value.
            (
                "https://gate.example.org/open?id=4&url=https%3A%2F%2Flms.example.edu%2Fpages%2F1%3Fverifier%3DDEMOSECRET",
                "https://gate.example.org/open?id=4",
            ),
            (
                "https://gate.example.org/open?url=https://lms.example.edu/pages/1?verifier=DEMOSECRET",
                "https://gate.example.org/open",
            ),
        ] {
            assert_eq!(scrub_text(address), clean, "{address}");
            let twice = scrub_text(address).into_owned();
            assert_eq!(scrub_text(&twice), twice, "{address}");
        }
    }

    #[test]
    fn only_the_address_goes_not_the_sentence_around_it() {
        // Chinese text follows an address without a space.
        assert_eq!(
            scrub_text(
                "请访问https://lms.example.edu/files/2/download?wrap=1下载讲义，并在周五前完成第三章的阅读。"
            ),
            "请访问https://lms.example.edu/files/2/download下载讲义，并在周五前完成第三章的阅读。"
        );
        assert_eq!(
            scrub_text("视频：https://media.example.edu/v?t=5&verifier=DEMOSECRET。之后做练习。"),
            "视频：https://media.example.edu/v?t=5。之后做练习。"
        );
        // Punctuation at the end of an address stays where it was.
        for (text, clean) in [
            (
                "See https://lms.example.edu/files/2/download?verifier=DEMOSECRET.",
                "See https://lms.example.edu/files/2/download.",
            ),
            (
                "Is it https://media.example.edu/v?t=5&verifier=DEMOSECRET? Yes: https://media.example.edu/w?access_token=DEMOSECRET, I think…",
                "Is it https://media.example.edu/v?t=5? Yes: https://media.example.edu/w, I think…",
            ),
            (
                "Slides (https://lms.example.edu/files/2/preview?verifier=DEMOSECRET); notes…",
                "Slides (https://lms.example.edu/files/2/preview); notes…",
            ),
        ] {
            assert_eq!(scrub_text(text), clean, "{text}");
        }
    }

    #[test]
    fn several_addresses_in_one_text_and_the_search_marks() {
        let text = "See notes (https://lms.example.edu/files/1/download?verifier=ONE) and\n\
                    the video https://media.example.edu/v?access_token=TWO&t=5 \"today\".";
        assert_eq!(
            scrub_text(text),
            "See notes (https://lms.example.edu/files/1/download) and\n\
             the video https://media.example.edu/v?t=5 \"today\"."
        );
        // A search snippet marks the match, inside the address: the parameter still goes.
        for snippet in [
            "…download?«verifier»=DEMOSECRET&wrap=1 and more…",
            "…watch?t=5&verifier=«DEMOSECRET» and more…",
            "…watch?t=5&access_«token»=DEMOSECRET and more…",
            "…/courses/1/«files»/7/download?verifier=DEMOSECRET) and more…",
        ] {
            assert!(!scrub_text(snippet).contains("DEMOSECRET"), "{snippet}");
        }
        assert_eq!(
            scrub_text("…watch?t=5&«verifier»=DEMOSECRET and more…"),
            "…watch?t=5 and more…"
        );
        let twice = scrub_text(text).into_owned();
        assert_eq!(scrub_text(&twice), twice);
    }

    #[test]
    fn a_parameter_goes_also_when_the_address_was_cut_before_it() {
        for (text, clean) in [
            // A PDF broke the line after the "?".
            (
                "https://lms.example.edu/courses/1/pages/week-3?\nverifier=DEMOSECRET&module_item_id=9 next",
                "https://lms.example.edu/courses/1/pages/week-3?\nmodule_item_id=9 next",
            ),
            // A search snippet that starts at the name.
            (
                "…«verifier»=DEMOSECRET&wrap=1) cover graphs",
                "…wrap=1) cover graphs",
            ),
            ("sf_verifier=DEMOSECRET", ""),
            ("t=5;access_token=DEMOSECRET, and", "t=5, and"),
        ] {
            assert_eq!(scrub_text(text), clean, "{text}");
        }
    }

    #[test]
    fn json_stays_json() {
        // An address right before an escaped quote or line break of a JSON string.
        let value = serde_json::json!({
            "text": "Open \"https://lms.example.edu/files/1/download?verifier=DEMOSECRET\" now\nor https://media.example.edu/v?access_token=DEMOSECRET\nlater",
        });
        let json = serde_json::to_string(&value).unwrap();
        let clean = scrub_text(&json);
        assert!(!clean.contains("DEMOSECRET"));
        let back: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(
            back["text"],
            "Open \"https://lms.example.edu/files/1/download\" now\nor https://media.example.edu/v\nlater"
        );
    }

    #[test]
    fn text_without_access_parameters_is_borrowed_and_unchanged() {
        for text in [
            "",
            "Plain text with a question? Yes.",
            "The verifier of a proof checks it; an access_token is a credential.",
            "https://lms.example.edu/courses/101/pages/week-3?module_item_id=9",
            "https://lms.example.edu/courses/101/files",
            "https://lms.example.edu/courses/101/files/folder/week?sort=name",
            "Which /files/ are due? All of them.",
            "x = verifier == 3 ? a : b",
            // Code that builds an address: a word starting with "?" isn't one.
            "url = base + \"?access_token=\" + token",
            "关于verifier的说明：见第三章？",
        ] {
            assert!(matches!(scrub_text(text), Cow::Borrowed(_)), "{text}");
        }
    }
}
