//! Content detection for documents (T030): text versus binary, the UTF-8 byte
//! order mark, the large-file threshold and the language id map.
//!
//! Only UTF-8 is supported. A file is text when it contains no NUL byte and
//! is valid UTF-8 as a whole; everything else is binary and never gets a
//! buffer, so it can never be corrupted by a save. UTF-16 files therefore show
//! up as binary (their NUL bytes), which is the documented limitation for now.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::path::WorkspacePath;

/// Files at or above this size open in large-file mode (read-only until the
/// UI asks to edit them); 10 MiB.
pub const LARGE_FILE_THRESHOLD: u64 = 10 * 1024 * 1024;
/// Files above this size are refused (`TooLarge`); 64 MiB. The whole content
/// crosses the IPC boundary as one string, so this bound protects the WebView.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const _: () = assert!(LARGE_FILE_THRESHOLD < MAX_DOCUMENT_BYTES);
pub const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// How the bytes of a document are interpreted. Serialized as
/// `"utf-8" | "utf-8-bom" | "binary"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum DocumentEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    /// UTF-8 with a byte order mark; the mark is stripped from the text the UI
    /// sees and written back on save.
    #[serde(rename = "utf-8-bom")]
    Utf8Bom,
    /// Not decodable as UTF-8 (or contains NUL bytes); no text buffer exists.
    #[serde(rename = "binary")]
    Binary,
}

impl DocumentEncoding {
    pub const fn is_text(self) -> bool {
        !matches!(self, Self::Binary)
    }

    pub const fn has_bom(self) -> bool {
        matches!(self, Self::Utf8Bom)
    }

    /// The inverse of `Display` / the wire form, for values read back from storage.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "utf-8" => Some(Self::Utf8),
            "utf-8-bom" => Some(Self::Utf8Bom),
            "binary" => Some(Self::Binary),
            _ => None,
        }
    }
}

impl fmt::Display for DocumentEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8-bom",
            Self::Binary => "binary",
        })
    }
}

/// Why a file was classified as binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryReason {
    NulByte,
    InvalidUtf8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Text { bom: bool },
    Binary { reason: BinaryReason },
}

impl ContentKind {
    pub const fn encoding(self) -> DocumentEncoding {
        match self {
            Self::Text { bom: false } => DocumentEncoding::Utf8,
            Self::Text { bom: true } => DocumentEncoding::Utf8Bom,
            Self::Binary { .. } => DocumentEncoding::Binary,
        }
    }
}

/// Classify a whole file. NUL bytes win over UTF-8 validity so a UTF-16 file
/// is reported as `NulByte` rather than as broken UTF-8.
pub fn detect_content(bytes: &[u8]) -> ContentKind {
    if bytes.contains(&0) {
        return ContentKind::Binary {
            reason: BinaryReason::NulByte,
        };
    }
    if std::str::from_utf8(bytes).is_err() {
        return ContentKind::Binary {
            reason: BinaryReason::InvalidUtf8,
        };
    }
    ContentKind::Text {
        bom: bytes.starts_with(&UTF8_BOM),
    }
}

/// The text of a file (byte order mark removed) and its encoding, or `None`
/// when the bytes are binary.
pub fn decode_text(bytes: &[u8]) -> Option<(String, DocumentEncoding)> {
    match detect_content(bytes) {
        ContentKind::Binary { .. } => None,
        kind @ ContentKind::Text { bom } => {
            let body = if bom { &bytes[UTF8_BOM.len()..] } else { bytes };
            // Validated by `detect_content`; the lossy path is never taken.
            Some((String::from_utf8_lossy(body).into_owned(), kind.encoding()))
        }
    }
}

/// The bytes to write for `text` in `encoding`. Line endings are kept as they
/// are in the text: the buffer is opaque, nothing is normalized.
pub fn encode_text(text: &str, encoding: DocumentEncoding) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + UTF8_BOM.len());
    if encoding.has_bom() {
        bytes.extend_from_slice(&UTF8_BOM);
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// `true` when a file of `size` bytes opens in large-file mode.
pub const fn is_large_file(size: u64) -> bool {
    size >= LARGE_FILE_THRESHOLD
}

/// Language identifier for a path, by exact file name first and then by
/// lower-cased extension. Identifiers follow the VS Code / Monaco vocabulary
/// (`docs/architecture/documents.md`); unknown files are `plaintext`.
pub fn language_id(path: &WorkspacePath) -> &'static str {
    let Some(name) = path.name() else {
        return PLAINTEXT;
    };
    if let Some((_, id)) = FILE_NAMES.iter().find(|(known, _)| *known == name) {
        return id;
    }
    let Some((_, extension)) = name.rsplit_once('.') else {
        return PLAINTEXT;
    };
    let extension = extension.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or(PLAINTEXT, |(_, id)| id)
}

pub const PLAINTEXT: &str = "plaintext";

/// Exact file names (case-sensitive) that override the extension.
pub const FILE_NAMES: &[(&str, &str)] = &[
    ("Dockerfile", "dockerfile"),
    ("Makefile", "makefile"),
    ("Cargo.lock", "toml"),
    ("CMakeLists.txt", "cmake"),
    (".gitignore", "ignore"),
    (".gitattributes", "ignore"),
    (".dockerignore", "ignore"),
    (".npmrc", "ini"),
    (".editorconfig", "ini"),
    (".env", "dotenv"),
];

/// Lower-cased extension → language id.
pub const EXTENSIONS: &[(&str, &str)] = &[
    ("ts", "typescript"),
    ("mts", "typescript"),
    ("cts", "typescript"),
    ("tsx", "typescript"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "javascript"),
    ("rs", "rust"),
    ("json", "json"),
    ("jsonc", "json"),
    ("md", "markdown"),
    ("markdown", "markdown"),
    ("toml", "toml"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("html", "html"),
    ("htm", "html"),
    ("css", "css"),
    ("scss", "scss"),
    ("less", "less"),
    ("py", "python"),
    ("go", "go"),
    ("java", "java"),
    ("kt", "kotlin"),
    ("swift", "swift"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("cc", "cpp"),
    ("cxx", "cpp"),
    ("hpp", "cpp"),
    ("hh", "cpp"),
    ("cs", "csharp"),
    ("rb", "ruby"),
    ("php", "php"),
    ("sh", "shell"),
    ("bash", "shell"),
    ("zsh", "shell"),
    ("ps1", "powershell"),
    ("sql", "sql"),
    ("xml", "xml"),
    ("svg", "xml"),
    ("plist", "xml"),
    ("ini", "ini"),
    ("txt", PLAINTEXT),
    ("log", PLAINTEXT),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_bytes_and_invalid_utf8_are_binary() {
        assert_eq!(
            detect_content(b"abc\0def"),
            ContentKind::Binary {
                reason: BinaryReason::NulByte
            }
        );
        assert_eq!(
            detect_content(b"caf\xe9"),
            ContentKind::Binary {
                reason: BinaryReason::InvalidUtf8
            }
        );
        // UTF-16LE "hi" with BOM: NUL bytes win.
        assert_eq!(
            detect_content(&[0xFF, 0xFE, b'h', 0, b'i', 0]),
            ContentKind::Binary {
                reason: BinaryReason::NulByte
            }
        );
        assert_eq!(detect_content(b"").encoding(), DocumentEncoding::Utf8);
        assert_eq!(
            detect_content("héllo 🎉\r\n".as_bytes()),
            ContentKind::Text { bom: false }
        );
        assert_eq!(
            detect_content(&[0xEF, 0xBB, 0xBF, b'x']),
            ContentKind::Text { bom: true }
        );
        // A lone BOM prefix that is followed by invalid bytes is still binary.
        assert!(matches!(
            detect_content(&[0xEF, 0xBB, 0xBF, 0xFF]),
            ContentKind::Binary { .. }
        ));
    }

    #[test]
    fn decode_strips_the_bom_and_encode_restores_it() {
        let (text, encoding) = decode_text(&[0xEF, 0xBB, 0xBF, b'a', b'\n']).unwrap();
        assert_eq!(text, "a\n");
        assert_eq!(encoding, DocumentEncoding::Utf8Bom);
        assert_eq!(
            encode_text(&text, encoding),
            [0xEF, 0xBB, 0xBF, b'a', b'\n']
        );
        let (text, encoding) = decode_text(b"a\r\nb").unwrap();
        assert_eq!(encoding, DocumentEncoding::Utf8);
        assert_eq!(encode_text(&text, encoding), b"a\r\nb");
        assert_eq!(decode_text(b"\0"), None);
        assert_eq!(encode_text("x", DocumentEncoding::Binary), b"x");
    }

    #[test]
    fn encoding_serializes_as_documented() {
        assert_eq!(
            serde_json::to_string(&DocumentEncoding::Utf8Bom).unwrap(),
            "\"utf-8-bom\""
        );
        let back: DocumentEncoding = serde_json::from_str("\"binary\"").unwrap();
        assert_eq!(back, DocumentEncoding::Binary);
        assert!(!back.is_text());
        assert_eq!(DocumentEncoding::Utf8.to_string(), "utf-8");
    }

    #[test]
    fn large_file_threshold_is_inclusive() {
        assert!(!is_large_file(LARGE_FILE_THRESHOLD - 1));
        assert!(is_large_file(LARGE_FILE_THRESHOLD));
    }

    #[test]
    fn language_ids_by_name_then_extension() {
        let id = |raw: &str| language_id(&WorkspacePath::parse(raw).unwrap());
        assert_eq!(id("src/main.rs"), "rust");
        assert_eq!(id("src/App.TSX"), "typescript");
        assert_eq!(id("README.md"), "markdown");
        assert_eq!(id("Cargo.lock"), "toml");
        assert_eq!(id("docker/Dockerfile"), "dockerfile");
        assert_eq!(id(".gitignore"), "ignore");
        assert_eq!(id("notes"), PLAINTEXT);
        assert_eq!(id("weird.unknownext"), PLAINTEXT);
        assert_eq!(id(".hidden"), PLAINTEXT);
        assert_eq!(id("a.b.json"), "json");
        assert_eq!(language_id(&WorkspacePath::root()), PLAINTEXT);
    }

    #[test]
    fn language_tables_have_no_duplicates() {
        for table in [FILE_NAMES, EXTENSIONS] {
            for (index, (key, _)) in table.iter().enumerate() {
                assert!(
                    !table[index + 1..].iter().any(|(other, _)| other == key),
                    "duplicate key {key}"
                );
            }
        }
        for (extension, _) in EXTENSIONS {
            assert_eq!(*extension, extension.to_ascii_lowercase());
        }
    }
}
