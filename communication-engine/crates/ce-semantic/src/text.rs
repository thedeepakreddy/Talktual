//! Tokenization and spoken-number parsing for the rule adapter.
//!
//! Spans are in Unicode scalar values (characters) of the input text, the
//! unit of [`ce_core::semantic::TextSpan`]. Tokens are lower-cased for
//! matching; mentions are always cut from the original text.

/// A token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    /// Folded form (lower case; leading/trailing punctuation removed).
    pub w: String,
    /// Start character (inclusive).
    pub start: u32,
    /// End character (exclusive).
    pub end: u32,
    /// The original token was followed by `?`.
    pub question_mark: bool,
}

/// Split into tokens on whitespace and hyphens; strip punctuation at token
/// edges (apostrophes, `$`, `%`, `:` and `.` inside digits are kept).
pub fn tokenize(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let n = chars.len();
    while i < n {
        while i < n && (chars[i].is_whitespace() || chars[i] == '-') {
            i += 1;
        }
        let s = i;
        while i < n && !(chars[i].is_whitespace() || chars[i] == '-') {
            i += 1;
        }
        if s == i {
            continue;
        }
        let mut a = s;
        let mut b = i;
        let keep_lead = |c: char| c.is_alphanumeric() || c == '$';
        let keep_trail = |c: char| c.is_alphanumeric() || c == '%';
        while a < b && !keep_lead(chars[a]) {
            a += 1;
        }
        let question_mark = chars[a.max(s)..i].contains(&'?');
        while b > a && !keep_trail(chars[b - 1]) {
            b -= 1;
        }
        if a == b {
            if let Some(last) = out.last_mut() {
                let last: &mut Tok = last;
                last.question_mark |= question_mark;
            }
            continue;
        }
        let w: String = chars[a..b].iter().flat_map(|c| c.to_lowercase()).collect();
        let w = w.replace('\u{2019}', "'");
        out.push(Tok {
            w,
            start: a as u32,
            end: b as u32,
            question_mark,
        });
    }
    out
}

/// Value of a unit word (0–19).
pub fn unit(w: &str) -> Option<u32> {
    Some(match w {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        _ => return None,
    })
}

/// Value of a tens word.
pub fn tens(w: &str) -> Option<u32> {
    Some(match w {
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn scale(w: &str) -> Option<u64> {
    Some(match w {
        "thousand" => 1_000,
        "million" => 1_000_000,
        "billion" => 1_000_000_000,
        _ => return None,
    })
}

/// Value of an ordinal word (1st–19th, tens).
pub fn ordinal_word(w: &str) -> Option<u32> {
    Some(match w {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "thirtieth" => 30,
        "fortieth" => 40,
        "fiftieth" => 50,
        "sixtieth" => 60,
        "seventieth" => 70,
        "eightieth" => 80,
        "ninetieth" => 90,
        "hundredth" => 100,
        _ => return None,
    })
}

/// A digit token's value: `25`, `2.5`, `1,000` (no sign, no exponent).
pub fn digits(w: &str) -> Option<f64> {
    if w.is_empty() || !w.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut seen_dot = false;
    let mut clean = String::new();
    for c in w.chars() {
        match c {
            '0'..='9' => clean.push(c),
            ',' => {}
            '.' if !seen_dot => {
                seen_dot = true;
                clean.push('.');
            }
            _ => return None,
        }
    }
    if clean.ends_with('.') || clean.len() > 15 {
        return None;
    }
    clean.parse().ok()
}

/// `1st`, `22nd`, `3rd`, `4th`.
pub fn digit_ordinal(w: &str) -> Option<u32> {
    let n = w.len();
    if n < 3 {
        return None;
    }
    let (num, suf) = w.split_at(n - 2);
    if !matches!(suf, "st" | "nd" | "rd" | "th") || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    num.parse().ok()
}

/// A parsed number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    /// Value.
    pub value: f64,
    /// Integral.
    pub integer: bool,
    /// Index one past the last token used.
    pub end: usize,
    /// Written with digits.
    pub digits: bool,
}

/// Parse a cardinal number starting at token `i`: digits, or number words
/// ("two hundred and five", "twenty five", "a thousand", "two point five").
/// Sequences of bare digits ("one two three") are not merged.
pub fn number(t: &[Tok], i: usize) -> Option<Num> {
    let w = t.get(i)?.w.as_str();
    if let Some(v) = digits(w) {
        return Some(Num {
            value: v,
            integer: v.fract() == 0.0 && !w.contains('.'),
            end: i + 1,
            digits: true,
        });
    }
    let mut total: u64 = 0;
    let mut current: u64 = 0;
    let mut j = i;
    // last: 0 none, 1 unit/teen, 2 tens, 3 tens+unit, 4 hundred, 5 scale
    let mut last = 0u8;
    let mut used = false;
    while let Some(tok) = t.get(j) {
        let w = tok.w.as_str();
        if (w == "a" || w == "an") && !used {
            // "a hundred", "a thousand"
            if t.get(j + 1)
                .is_some_and(|n| n.w == "hundred" || scale(&n.w).is_some())
            {
                current = 1;
                last = 1;
                used = true;
                j += 1;
                continue;
            }
            break;
        }
        if let Some(u) = unit(w) {
            if last == 1 || last == 3 || (last == 2 && u >= 10) {
                break;
            }
            current += u64::from(u);
            last = if last == 2 { 3 } else { 1 };
        } else if let Some(tn) = tens(w) {
            if matches!(last, 1..=3) {
                break;
            }
            current += u64::from(tn);
            last = 2;
        } else if w == "hundred" {
            if !matches!(last, 1..=3) || current >= 100 {
                break;
            }
            current *= 100;
            last = 4;
        } else if let Some(sc) = scale(w) {
            if last == 0 || last == 5 || current == 0 {
                break;
            }
            total += current * sc;
            current = 0;
            last = 5;
        } else if w == "and" && matches!(last, 4 | 5) {
            if t.get(j + 1)
                .is_some_and(|n| unit(&n.w).is_some() || tens(&n.w).is_some())
            {
                j += 1;
                continue;
            }
            break;
        } else {
            break;
        }
        used = true;
        j += 1;
    }
    if !used || (last == 0) {
        return None;
    }
    let mut value = (total + current) as f64;
    let mut integer = true;
    // Decimal: "point five", "point two five".
    if t.get(j).is_some_and(|p| p.w == "point") {
        let mut k = j + 1;
        let mut frac = String::new();
        while let Some(d) = t.get(k).and_then(|x| unit(&x.w).filter(|u| *u < 10)) {
            frac.push(char::from(b'0' + d as u8));
            k += 1;
        }
        if !frac.is_empty() {
            if let Ok(f) = format!("0.{frac}").parse::<f64>() {
                value += f;
                integer = false;
                j = k;
            }
        }
    }
    Some(Num {
        value,
        integer,
        end: j,
        digits: false,
    })
}

/// Parse an ordinal at `i`: `third`, `twenty first`, `21st`.
pub fn ordinal(t: &[Tok], i: usize) -> Option<(u32, usize)> {
    let w = t.get(i)?.w.as_str();
    if let Some(v) = digit_ordinal(w) {
        return Some((v, i + 1));
    }
    if let Some(v) = ordinal_word(w) {
        return Some((v, i + 1));
    }
    let tn = tens(w)?;
    let o = t
        .get(i + 1)
        .and_then(|n| ordinal_word(&n.w))
        .filter(|o| *o < 10)?;
    Some((tn + o, i + 2))
}

/// A single spoken digit (`oh` counts as zero) or a one-character digit.
pub fn single_digit(w: &str) -> Option<u32> {
    if w == "oh" || w == "o" {
        return Some(0);
    }
    if w.len() == 1 {
        return w.chars().next().and_then(|c| c.to_digit(10));
    }
    unit(w).filter(|u| *u < 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Option<(f64, usize)> {
        let t = tokenize(s);
        number(&t, 0).map(|x| (x.value, x.end))
    }

    #[test]
    fn tokens_keep_character_spans() {
        let t = tokenize("Hello, wörld?  twenty-five $5 9:30.");
        let words: Vec<_> = t.iter().map(|x| x.w.as_str()).collect();
        assert_eq!(words, ["hello", "wörld", "twenty", "five", "$5", "9:30"]);
        assert_eq!((t[1].start, t[1].end), (7, 12));
        assert!(t[1].question_mark);
        assert_eq!((t[3].start, t[3].end), (22, 26));
    }

    #[test]
    fn spoken_numbers() {
        assert_eq!(n("twenty five"), Some((25.0, 2)));
        assert_eq!(n("two hundred and five"), Some((205.0, 4)));
        assert_eq!(n("a thousand"), Some((1000.0, 2)));
        assert_eq!(n("three thousand four hundred"), Some((3400.0, 4)));
        assert_eq!(
            n("one two three"),
            Some((1.0, 1)),
            "digit runs stay separate"
        );
        assert_eq!(n("two point five"), Some((2.5, 3)));
        assert_eq!(n("1,250"), Some((1250.0, 1)));
        assert_eq!(n("nineteen ninety"), Some((19.0, 1)));
        assert_eq!(n("twenty thirty"), Some((20.0, 1)));
        assert_eq!(n("a box"), None);
        assert_eq!(n("and"), None);
        let t = tokenize("twenty first 3rd");
        assert_eq!(ordinal(&t, 0), Some((21, 2)));
        assert_eq!(ordinal(&t, 2), Some((3, 3)));
    }
}
