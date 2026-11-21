//! Text of edit controls. Ordinary text is a `String`; secret text
//! (passwords) lives in a buffer that is wiped whenever it is replaced and
//! when it is dropped. The undo history of egui, which keeps plain copies,
//! is cleared for secret fields after every frame (see `render`).

use egui::text::CharIndex;
use zeroize::Zeroizing;

pub enum TextBuf {
    Plain(String),
    Secret(Zeroizing<String>),
}

impl TextBuf {
    pub fn plain(s: &str) -> Self {
        TextBuf::Plain(s.to_string())
    }

    pub fn is_secret(&self) -> bool {
        matches!(self, TextBuf::Secret(_))
    }

    /// Keep the text in a wiped buffer from now on.
    pub fn make_secret(&mut self) {
        if let TextBuf::Plain(s) = self {
            let secret = Zeroizing::new(std::mem::take(s));
            *self = TextBuf::Secret(secret);
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            TextBuf::Plain(s) => s,
            TextBuf::Secret(s) => s,
        }
    }

    pub fn set(&mut self, text: &str) {
        match self {
            TextBuf::Plain(s) => {
                s.clear();
                s.push_str(text);
            }
            TextBuf::Secret(s) => *s = Zeroizing::new(text.to_string()),
        }
    }
}

fn byte_of(s: &str, char_index: usize) -> usize {
    s.char_indices().nth(char_index).map_or(s.len(), |(i, _)| i)
}

impl egui::TextBuffer for TextBuf {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        TextBuf::as_str(self)
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        match self {
            TextBuf::Plain(s) => {
                let b = byte_of(s, char_index.0);
                s.insert_str(b, text);
            }
            TextBuf::Secret(s) => {
                // build the new text in a fresh buffer: growing the old one
                // could leave a copy in freed memory
                let b = byte_of(s, char_index.0);
                let mut n = Zeroizing::new(String::with_capacity((s.len() + text.len()).max(64)));
                n.push_str(&s[..b]);
                n.push_str(text);
                n.push_str(&s[b..]);
                *s = n;
            }
        }
        text.chars().count()
    }

    fn delete_char_range(&mut self, r: std::ops::Range<CharIndex>) {
        match self {
            TextBuf::Plain(s) => {
                let (a, b) = (byte_of(s, r.start.0), byte_of(s, r.end.0));
                s.replace_range(a..b, "");
            }
            TextBuf::Secret(s) => {
                let (a, b) = (byte_of(s, r.start.0), byte_of(s, r.end.0));
                let mut n = Zeroizing::new(String::with_capacity(s.capacity()));
                n.push_str(&s[..a]);
                n.push_str(&s[b..]);
                *s = n;
            }
        }
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<TextBuf>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::TextBuffer;

    #[test]
    fn edit_both_kinds() {
        for secret in [false, true] {
            let mut t = TextBuf::plain("aЖc");
            if secret {
                t.make_secret();
            }
            assert_eq!(t.is_secret(), secret);
            t.insert_text("1", CharIndex(1));
            assert_eq!(TextBuf::as_str(&t), "a1Жc");
            t.delete_char_range(CharIndex(2)..CharIndex(3));
            assert_eq!(TextBuf::as_str(&t), "a1c");
            t.set("x");
            assert_eq!(TextBuf::as_str(&t), "x");
        }
    }
}
