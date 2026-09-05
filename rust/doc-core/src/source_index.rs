//! Index over source file paths and content, tracking staleness via file metadata.
//!
//! Provides an in-memory index of source files that can be queried by token, and tracks
//! staleness by monitoring file modification times and content hashes. Designed for
//! environments (like wasm) where the host controls file I/O — the module accepts metadata
//! and content as input, never performs I/O itself.

use std::collections::HashMap;

/// Metadata about a source file: its path, modification time, and content hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMetadata {
    /// The file path, relative or absolute.
    pub path: String,
    /// File modification time as a Unix timestamp (seconds since epoch).
    pub mtime: u64,
    /// SHA-256 hex digest of the file's content.
    pub content_hash: String,
}

/// A queryable index of source files and their tokens.
///
/// The index tracks file metadata and tokenizes content for fast lookup. Staleness is
/// detected by comparing stored metadata (mtime + content hash) against current file state.
#[derive(Clone, Debug)]
pub struct SourceIndex {
    /// Map of file path → metadata (mtime + content hash).
    metadata: HashMap<String, (u64, String)>,
    /// Map of token → set of file paths that contain it.
    token_index: HashMap<String, Vec<String>>,
}

impl SourceIndex {
    /// Creates a new, empty source index.
    pub fn new() -> Self {
        SourceIndex {
            metadata: HashMap::new(),
            token_index: HashMap::new(),
        }
    }

    /// Builds an index from a list of files with their content.
    ///
    /// Each file's content is tokenized (lowercased, split on non-alphanumeric chars,
    /// compound identifiers broken into parts, empty tokens dropped). The file's
    /// modification time and content hash are stored to later detect staleness.
    ///
    /// Returns an error if tokenization fails (e.g., invalid UTF-8).
    pub fn build_from(
        files: Vec<(FileMetadata, String)>,
    ) -> Result<SourceIndex, String> {
        let mut index = SourceIndex::new();

        for (metadata, content) in files {
            let tokens = tokenize(&content)?;
            index.metadata.insert(
                metadata.path.clone(),
                (metadata.mtime, metadata.content_hash.clone()),
            );

            for token in tokens {
                index
                    .token_index
                    .entry(token)
                    .or_insert_with(Vec::new)
                    .push(metadata.path.clone());
            }
        }

        Ok(index)
    }

    /// Returns an iterator over the indexed file metadata (path → (mtime, hash)).
    pub fn metadata_iter(&self) -> impl Iterator<Item = (&String, &(u64, String))> {
        self.metadata.iter()
    }

    /// Queries the index for files containing a given token.
    ///
    /// Returns a list of file paths (in the order they were added) that contain the token.
    /// Returns an empty list if the token is not found.
    pub fn query(&self, token: &str) -> Vec<String> {
        self.token_index
            .get(&token.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// Detects whether the index is stale relative to current file metadata.
    ///
    /// Returns `Ok(true)` if any file's mtime or content_hash has changed, or if a
    /// previously-indexed file is missing. Returns `Ok(false)` if all files are unchanged.
    /// Returns an error if the current metadata is malformed.
    pub fn is_stale(&self, current_files: &[FileMetadata]) -> Result<bool, String> {
        let current_map: HashMap<String, (u64, String)> = current_files
            .iter()
            .map(|f| (f.path.clone(), (f.mtime, f.content_hash.clone())))
            .collect();

        // Check if any previously-indexed file is missing or has changed.
        for (path, (old_mtime, old_hash)) in &self.metadata {
            match current_map.get(path) {
                None => return Ok(true), // File was removed.
                Some((new_mtime, new_hash)) => {
                    if old_mtime != new_mtime || old_hash != new_hash {
                        return Ok(true); // File was modified.
                    }
                }
            }
        }

        // Check if any new files were added.
        if current_map.len() != self.metadata.len() {
            return Ok(true);
        }

        Ok(false)
    }

    /// Serializes the index to bytes for storage.
    ///
    /// The format is a simple binary encoding: file count (u32), then for each file:
    /// - path length (u32) + path bytes
    /// - mtime (u64)
    /// - content_hash length (u32) + content_hash bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Write metadata count.
        bytes.extend((self.metadata.len() as u32).to_le_bytes());

        for (path, (mtime, hash)) in &self.metadata {
            // Write path.
            bytes.extend((path.len() as u32).to_le_bytes());
            bytes.extend(path.as_bytes());

            // Write mtime.
            bytes.extend(mtime.to_le_bytes());

            // Write hash.
            bytes.extend((hash.len() as u32).to_le_bytes());
            bytes.extend(hash.as_bytes());
        }

        bytes
    }

    /// Deserializes an index from bytes.
    ///
    /// Expects the format produced by [`to_bytes`]. Returns an error if the bytes
    /// are truncated or malformed.
    pub fn from_bytes(bytes: &[u8]) -> Result<SourceIndex, String> {
        if bytes.len() < 4 {
            return Err("truncated: need at least 4 bytes for count".to_string());
        }

        let mut pos = 0;
        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        pos += 4;

        let mut metadata = HashMap::new();

        for _ in 0..count {
            // Read path.
            if pos + 4 > bytes.len() {
                return Err("truncated: cannot read path length".to_string());
            }
            let path_len =
                u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                    as usize;
            pos += 4;

            if pos + path_len > bytes.len() {
                return Err("truncated: cannot read path bytes".to_string());
            }
            let path = String::from_utf8(bytes[pos..pos + path_len].to_vec())
                .map_err(|e| format!("invalid UTF-8 in path: {}", e))?;
            pos += path_len;

            // Read mtime.
            if pos + 8 > bytes.len() {
                return Err("truncated: cannot read mtime".to_string());
            }
            let mtime = u64::from_le_bytes([
                bytes[pos],
                bytes[pos + 1],
                bytes[pos + 2],
                bytes[pos + 3],
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ]);
            pos += 8;

            // Read hash.
            if pos + 4 > bytes.len() {
                return Err("truncated: cannot read hash length".to_string());
            }
            let hash_len =
                u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                    as usize;
            pos += 4;

            if pos + hash_len > bytes.len() {
                return Err("truncated: cannot read hash bytes".to_string());
            }
            let hash = String::from_utf8(bytes[pos..pos + hash_len].to_vec())
                .map_err(|e| format!("invalid UTF-8 in hash: {}", e))?;
            pos += hash_len;

            metadata.insert(path, (mtime, hash));
        }

        Ok(SourceIndex {
            metadata,
            token_index: HashMap::new(),
        })
    }
}

impl Default for SourceIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenizes a string: splits on non-alphanumeric chars, extracts camelCase/snake_case parts,
/// lowercases, and drops empty tokens.
fn tokenize(content: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();

    for word in content.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }

        let lowercased_word = word.to_lowercase();
        // Add the whole word (lowercased).
        tokens.push(lowercased_word.clone());

        // Extract camelCase and snake_case parts using the original-cased word.
        let parts = extract_parts(word);
        for part in parts {
            let lowercased_part = part.to_lowercase();
            if !lowercased_part.is_empty() && lowercased_part != lowercased_word {
                tokens.push(lowercased_part);
            }
        }
    }

    Ok(tokens)
}

/// Extracts parts from a camelCase or snake_case identifier.
fn extract_parts(word: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;

    for ch in word.chars() {
        if ch == '_' {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            prev_lower = false;
        } else if ch.is_uppercase() && prev_lower {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            current.push(ch.to_lowercase().next().unwrap());
            prev_lower = true;
        } else {
            current.push(ch);
            prev_lower = ch.is_lowercase();
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_empty_index() {
        let result = SourceIndex::build_from(vec![]);
        assert!(result.is_ok());
        let index = result.unwrap();
        assert_eq!(index.query("any_token"), Vec::<String>::new());
    }

    #[test]
    fn test_build_single_file() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let content = "hello world".to_string();
        let result = SourceIndex::build_from(vec![(meta, content)]);

        assert!(result.is_ok());
        let index = result.unwrap();
        assert_eq!(index.query("hello"), vec!["test.rs"]);
        assert_eq!(index.query("world"), vec!["test.rs"]);
    }

    #[test]
    fn test_tokenization_lowercase() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let content = "Hello WORLD Rust".to_string();
        let result = SourceIndex::build_from(vec![(meta, content)]);

        let index = result.unwrap();
        assert_eq!(index.query("hello"), vec!["test.rs"]);
        assert_eq!(index.query("HELLO"), vec!["test.rs"]);
        assert_eq!(index.query("world"), vec!["test.rs"]);
    }

    #[test]
    fn test_tokenization_split_on_non_alphanumeric() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let content = "foo-bar_baz.qux:quux".to_string();
        let result = SourceIndex::build_from(vec![(meta, content)]);

        let index = result.unwrap();
        assert_eq!(index.query("foo"), vec!["test.rs"]);
        assert_eq!(index.query("bar"), vec!["test.rs"]);
        assert_eq!(index.query("baz"), vec!["test.rs"]);
        assert_eq!(index.query("qux"), vec!["test.rs"]);
        assert_eq!(index.query("quux"), vec!["test.rs"]);
    }

    #[test]
    fn test_tokenization_empty_tokens_dropped() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let content = "a   b---c".to_string();
        let result = SourceIndex::build_from(vec![(meta, content)]);

        let index = result.unwrap();
        assert_eq!(index.query("a"), vec!["test.rs"]);
        assert_eq!(index.query("b"), vec!["test.rs"]);
        assert_eq!(index.query("c"), vec!["test.rs"]);
    }

    #[test]
    fn test_multiple_files_same_token() {
        let meta1 = FileMetadata {
            path: "file1.rs".to_string(),
            mtime: 100,
            content_hash: "hash1".to_string(),
        };
        let meta2 = FileMetadata {
            path: "file2.rs".to_string(),
            mtime: 200,
            content_hash: "hash2".to_string(),
        };
        let result = SourceIndex::build_from(vec![
            (meta1, "hello world".to_string()),
            (meta2, "hello rust".to_string()),
        ]);

        let index = result.unwrap();
        let mut results = index.query("hello");
        results.sort();
        assert_eq!(results, vec!["file1.rs", "file2.rs"]);
    }

    #[test]
    fn test_token_not_found() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let result = SourceIndex::build_from(vec![(meta, "hello".to_string())]);

        let index = result.unwrap();
        assert_eq!(index.query("nonexistent"), Vec::<String>::new());
    }

    #[test]
    fn test_is_stale_no_files() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta, "hello".to_string())]).unwrap();

        let result = index.is_stale(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_is_stale_mtime_changed() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta, "hello".to_string())]).unwrap();

        let changed_meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 200,
            content_hash: "abc123".to_string(),
        };
        let result = index.is_stale(&[changed_meta]);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_is_stale_hash_changed() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta, "hello".to_string())]).unwrap();

        let changed_meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "xyz789".to_string(),
        };
        let result = index.is_stale(&[changed_meta]);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_is_stale_file_added() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta, "hello".to_string())]).unwrap();

        let current = vec![
            FileMetadata {
                path: "test.rs".to_string(),
                mtime: 100,
                content_hash: "abc123".to_string(),
            },
            FileMetadata {
                path: "new.rs".to_string(),
                mtime: 150,
                content_hash: "newHash".to_string(),
            },
        ];
        let result = index.is_stale(&current);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_is_not_stale() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta.clone(), "hello".to_string())]).unwrap();

        let result = index.is_stale(&[meta]);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_serialization_roundtrip_empty() {
        let index = SourceIndex::new();
        let bytes = index.to_bytes();
        let result = SourceIndex::from_bytes(&bytes);

        assert!(result.is_ok());
        let restored = result.unwrap();
        assert_eq!(restored.metadata.len(), 0);
    }

    #[test]
    fn test_serialization_roundtrip_single_file() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta.clone(), "hello".to_string())]).unwrap();

        let bytes = index.to_bytes();
        let result = SourceIndex::from_bytes(&bytes);

        assert!(result.is_ok());
        let restored = result.unwrap();
        assert_eq!(restored.metadata.len(), 1);
        assert_eq!(
            restored.metadata.get("test.rs"),
            Some(&(100, "abc123".to_string()))
        );
    }

    #[test]
    fn test_serialization_roundtrip_multiple_files() {
        let meta1 = FileMetadata {
            path: "file1.rs".to_string(),
            mtime: 100,
            content_hash: "hash1".to_string(),
        };
        let meta2 = FileMetadata {
            path: "file2.rs".to_string(),
            mtime: 200,
            content_hash: "hash2".to_string(),
        };
        let index = SourceIndex::build_from(vec![
            (meta1, "hello".to_string()),
            (meta2, "world".to_string()),
        ])
        .unwrap();

        let bytes = index.to_bytes();
        let result = SourceIndex::from_bytes(&bytes);

        assert!(result.is_ok());
        let restored = result.unwrap();
        assert_eq!(restored.metadata.len(), 2);
        assert_eq!(
            restored.metadata.get("file1.rs"),
            Some(&(100, "hash1".to_string()))
        );
        assert_eq!(
            restored.metadata.get("file2.rs"),
            Some(&(200, "hash2".to_string()))
        );
    }

    #[test]
    fn test_deserialization_truncated() {
        let bytes = vec![5, 0, 0, 0];
        let result = SourceIndex::from_bytes(&bytes);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("truncated"));
    }

    #[test]
    fn test_deserialization_empty_bytes() {
        let bytes = vec![];
        let result = SourceIndex::from_bytes(&bytes);

        assert!(result.is_err());
    }

    #[test]
    fn test_deserialization_utf8_path() {
        let meta = FileMetadata {
            path: "файл.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta, "hello".to_string())]).unwrap();

        let bytes = index.to_bytes();
        let result = SourceIndex::from_bytes(&bytes);

        assert!(result.is_ok());
        let restored = result.unwrap();
        assert!(restored.metadata.contains_key("файл.rs"));
    }

    #[test]
    fn test_query_case_insensitive() {
        let meta = FileMetadata {
            path: "test.rs".to_string(),
            mtime: 100,
            content_hash: "abc123".to_string(),
        };
        let index = SourceIndex::build_from(vec![(meta, "HelloWorld".to_string())]).unwrap();

        assert_eq!(index.query("hello"), vec!["test.rs"]);
        assert_eq!(index.query("HELLO"), vec!["test.rs"]);
        assert_eq!(index.query("HeLLo"), vec!["test.rs"]);
    }

    #[test]
    fn test_default_creates_empty_index() {
        let index = SourceIndex::default();
        assert_eq!(index.query("anything"), Vec::<String>::new());
    }
}
