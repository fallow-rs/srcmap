//! React Native RAM bundle parser for source map tooling.
//!
//! Supports two RAM bundle formats used by React Native / Metro:
//!
//! - **Indexed RAM bundles** (iOS): single binary file with magic number `0xFB0BD1E5`
//! - **Unbundles** (Android): directory-based with `js-modules/` structure
//!
//! # Examples
//!
//! ```
//! use srcmap_ram_bundle::{IndexedRamBundle, is_ram_bundle};
//!
//! // Build a minimal RAM bundle for demonstration
//! let startup = b"var startup = true;";
//! let module0 = b"__d(function(){},0);";
//!
//! let mut data = Vec::new();
//! // Magic number
//! data.extend_from_slice(&0xFB0BD1E5_u32.to_le_bytes());
//! // Module count: 1
//! data.extend_from_slice(&1_u32.to_le_bytes());
//! // Startup code size
//! data.extend_from_slice(&(startup.len() as u32).to_le_bytes());
//! // Module table entry: offset 0, length of module0
//! data.extend_from_slice(&0_u32.to_le_bytes());
//! data.extend_from_slice(&(module0.len() as u32).to_le_bytes());
//! // Startup code
//! data.extend_from_slice(startup);
//! // Module code
//! data.extend_from_slice(module0);
//!
//! assert!(is_ram_bundle(&data));
//!
//! let bundle = IndexedRamBundle::from_bytes(&data).unwrap();
//! assert_eq!(bundle.startup_code(), "var startup = true;");
//! assert_eq!(bundle.module_count(), 1);
//! assert_eq!(bundle.get_module(0).unwrap().source_code, "__d(function(){},0);");
//! ```

use std::fmt;
use std::path::Path;

/// Magic number for indexed RAM bundles (iOS format).
const RAM_BUNDLE_MAGIC: u32 = 0xFB0BD1E5;

/// Size of the fixed header: magic (4) + module_count (4) + startup_code_size (4).
const HEADER_SIZE: usize = 12;

/// Size of each module table entry: offset (4) + length (4).
const MODULE_ENTRY_SIZE: usize = 8;

/// Error type for RAM bundle operations.
#[derive(Debug)]
pub enum RamBundleError {
    /// Invalid magic number.
    InvalidMagic,
    /// Data too short to contain a valid header.
    TooShort,
    /// Invalid module entry.
    InvalidEntry(String),
    /// I/O error.
    Io(std::io::Error),
    /// Source map parse error.
    SourceMap(srcmap_sourcemap::ParseError),
}

impl fmt::Display for RamBundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "invalid RAM bundle magic number"),
            Self::TooShort => write!(f, "data too short for RAM bundle header"),
            Self::InvalidEntry(msg) => write!(f, "invalid module entry: {msg}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::SourceMap(e) => write!(f, "source map error: {e}"),
        }
    }
}

impl std::error::Error for RamBundleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::SourceMap(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for RamBundleError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<srcmap_sourcemap::ParseError> for RamBundleError {
    fn from(e: srcmap_sourcemap::ParseError) -> Self {
        Self::SourceMap(e)
    }
}

/// Type of RAM bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamBundleType {
    /// Indexed format (iOS) - single binary file.
    Indexed,
    /// Unbundle format (Android) - directory with `js-modules/`.
    Unbundle,
}

/// A parsed RAM bundle module.
#[derive(Debug, Clone)]
pub struct RamBundleModule {
    /// Module ID (0-based index).
    pub id: u32,
    /// Module source code.
    pub source_code: String,
}

/// A parsed indexed RAM bundle.
///
/// The indexed format is a single binary file used primarily on iOS. It contains
/// a header with a module table followed by startup code and module source code.
#[derive(Debug)]
pub struct IndexedRamBundle {
    /// Number of modules in the bundle.
    pub module_count: u32,
    /// Startup (prelude) code that runs before modules.
    pub startup_code: String,
    /// Individual modules indexed by ID.
    modules: Vec<Option<RamBundleModule>>,
}

impl IndexedRamBundle {
    /// Parse an indexed RAM bundle from raw bytes.
    ///
    /// The binary layout is:
    /// - Bytes 0..4: magic number (little-endian `u32`) = `0xFB0BD1E5`
    /// - Bytes 4..8: module count (little-endian `u32`)
    /// - Bytes 8..12: startup code size (little-endian `u32`)
    /// - Next `module_count * 8` bytes: module table entries (offset + length, each `u32` LE)
    /// - Startup code (UTF-8)
    /// - Module source code at specified offsets (UTF-8)
    pub fn from_bytes(data: &[u8]) -> Result<Self, RamBundleError> {
        if data.len() < HEADER_SIZE {
            return Err(RamBundleError::TooShort);
        }

        let magic = read_u32_le(data, 0).unwrap();
        if magic != RAM_BUNDLE_MAGIC {
            return Err(RamBundleError::InvalidMagic);
        }

        let module_count = read_u32_le(data, 4).unwrap();
        let startup_code_size = read_u32_le(data, 8).unwrap() as usize;

        let table_size = (module_count as usize)
            .checked_mul(MODULE_ENTRY_SIZE)
            .ok_or(RamBundleError::TooShort)?;
        let table_end = HEADER_SIZE
            .checked_add(table_size)
            .ok_or(RamBundleError::TooShort)?;

        if data.len() < table_end {
            return Err(RamBundleError::TooShort);
        }

        // Startup code comes right after the module table
        let startup_start = table_end;
        let startup_end = startup_start
            .checked_add(startup_code_size)
            .ok_or(RamBundleError::TooShort)?;

        if data.len() < startup_end {
            return Err(RamBundleError::TooShort);
        }

        let startup_code = std::str::from_utf8(&data[startup_start..startup_end])
            .map_err(|e| {
                RamBundleError::InvalidEntry(format!("startup code is not valid UTF-8: {e}"))
            })?
            .to_owned();

        // The base offset for module data is right after startup code
        let modules_base = startup_end;

        let mut modules = Vec::with_capacity(module_count as usize);

        for i in 0..module_count as usize {
            let entry_offset = HEADER_SIZE + i * MODULE_ENTRY_SIZE;
            let offset = read_u32_le(data, entry_offset).unwrap() as usize;
            let length = read_u32_le(data, entry_offset + 4).unwrap() as usize;

            if offset == 0 && length == 0 {
                modules.push(None);
                continue;
            }

            let abs_start = modules_base.checked_add(offset).ok_or_else(|| {
                RamBundleError::InvalidEntry(format!("module {i} offset overflows"))
            })?;
            let abs_end = abs_start.checked_add(length).ok_or_else(|| {
                RamBundleError::InvalidEntry(format!("module {i} length overflows"))
            })?;

            if abs_end > data.len() {
                return Err(RamBundleError::InvalidEntry(format!(
                    "module {i} extends beyond data (offset={offset}, length={length}, data_len={})",
                    data.len()
                )));
            }

            let source_code = std::str::from_utf8(&data[abs_start..abs_end])
                .map_err(|e| {
                    RamBundleError::InvalidEntry(format!(
                        "module {i} source is not valid UTF-8: {e}"
                    ))
                })?
                .to_owned();

            modules.push(Some(RamBundleModule {
                id: i as u32,
                source_code,
            }));
        }

        Ok(Self {
            module_count,
            startup_code,
            modules,
        })
    }

    /// Returns the number of module slots in the bundle.
    pub fn module_count(&self) -> u32 {
        self.module_count
    }

    /// Returns a module by its ID, or `None` if the slot is empty.
    pub fn get_module(&self, id: u32) -> Option<&RamBundleModule> {
        self.modules.get(id as usize)?.as_ref()
    }

    /// Iterates over all non-empty modules in the bundle.
    pub fn modules(&self) -> impl Iterator<Item = &RamBundleModule> {
        self.modules.iter().filter_map(|m| m.as_ref())
    }

    /// Returns the startup (prelude) code.
    pub fn startup_code(&self) -> &str {
        &self.startup_code
    }
}

/// Check if data starts with the RAM bundle magic number.
///
/// Requires at least 4 bytes.
pub fn is_ram_bundle(data: &[u8]) -> bool {
    read_u32_le(data, 0) == Some(RAM_BUNDLE_MAGIC)
}

/// Check if a path looks like an unbundle (file RAM bundle) directory.
///
/// Returns `true` if the path contains a `js-modules` subdirectory.
pub fn is_unbundle_dir(path: &Path) -> bool {
    path.join("js-modules").is_dir()
}

/// Read a little-endian `u32` from `data` at the given byte offset.
fn read_u32_le(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid indexed RAM bundle from module source strings.
    ///
    /// `modules` is a slice of optional source code strings. `None` entries
    /// produce empty module table slots (offset=0, length=0).
    fn make_test_bundle(modules: &[Option<&str>], startup: &str) -> Vec<u8> {
        let mut data = Vec::new();

        // Header
        data.extend_from_slice(&RAM_BUNDLE_MAGIC.to_le_bytes());
        data.extend_from_slice(&(modules.len() as u32).to_le_bytes());
        data.extend_from_slice(&(startup.len() as u32).to_le_bytes());

        // Build module bodies and compute offsets.
        // Offsets are relative to the start of the module data section
        // (which comes after the header + table + startup code).
        let mut module_bodies: Vec<(u32, u32)> = Vec::new();
        let mut current_offset: u32 = 0;

        for module in modules {
            match module {
                Some(src) => {
                    let len = src.len() as u32;
                    module_bodies.push((current_offset, len));
                    current_offset += len;
                }
                None => {
                    module_bodies.push((0, 0));
                }
            }
        }

        // Module table
        for &(offset, length) in &module_bodies {
            data.extend_from_slice(&offset.to_le_bytes());
            data.extend_from_slice(&length.to_le_bytes());
        }

        // Startup code
        data.extend_from_slice(startup.as_bytes());

        // Module source code
        for module in modules.iter().flatten() {
            data.extend_from_slice(module.as_bytes());
        }

        data
    }

    #[test]
    fn test_is_ram_bundle() {
        let data = make_test_bundle(&[], "");
        assert!(is_ram_bundle(&data));
    }

    #[test]
    fn test_is_ram_bundle_wrong_magic() {
        let data = [0x00, 0x00, 0x00, 0x00];
        assert!(!is_ram_bundle(&data));
    }

    #[test]
    fn test_is_ram_bundle_too_short() {
        assert!(!is_ram_bundle(&[0xE5, 0xD1, 0x0B]));
        assert!(!is_ram_bundle(&[]));
    }

    #[test]
    fn test_parse_empty_bundle() {
        let data = make_test_bundle(&[], "var x = 1;");
        let bundle = IndexedRamBundle::from_bytes(&data).unwrap();
        assert_eq!(bundle.module_count(), 0);
        assert_eq!(bundle.startup_code(), "var x = 1;");
        assert_eq!(bundle.modules().count(), 0);
    }

    #[test]
    fn test_parse_single_module() {
        let data = make_test_bundle(&[Some("__d(function(){},0);")], "startup();");
        let bundle = IndexedRamBundle::from_bytes(&data).unwrap();

        assert_eq!(bundle.module_count(), 1);
        assert_eq!(bundle.startup_code(), "startup();");

        let module = bundle.get_module(0).unwrap();
        assert_eq!(module.id, 0);
        assert_eq!(module.source_code, "__d(function(){},0);");
    }

    #[test]
    fn test_parse_multiple_modules() {
        let modules = vec![
            Some("__d(function(){console.log('a')},0);"),
            Some("__d(function(){console.log('b')},1);"),
            Some("__d(function(){console.log('c')},2);"),
        ];
        let data = make_test_bundle(&modules, "require(0);");
        let bundle = IndexedRamBundle::from_bytes(&data).unwrap();

        assert_eq!(bundle.module_count(), 3);
        assert_eq!(bundle.startup_code(), "require(0);");

        for (i, module) in bundle.modules().enumerate() {
            assert_eq!(module.id, i as u32);
            assert!(
                module
                    .source_code
                    .contains(&format!("'{}'", (b'a' + i as u8) as char))
            );
        }
    }

    #[test]
    fn test_empty_module_slots() {
        let modules = vec![
            Some("__d(function(){},0);"),
            None,
            Some("__d(function(){},2);"),
        ];
        let data = make_test_bundle(&modules, "");
        let bundle = IndexedRamBundle::from_bytes(&data).unwrap();

        assert_eq!(bundle.module_count(), 3);
        assert!(bundle.get_module(0).is_some());
        assert!(bundle.get_module(1).is_none());
        assert!(bundle.get_module(2).is_some());

        // Only 2 non-empty modules
        assert_eq!(bundle.modules().count(), 2);
    }

    #[test]
    fn test_get_module_out_of_range() {
        let data = make_test_bundle(&[Some("__d(function(){},0);")], "");
        let bundle = IndexedRamBundle::from_bytes(&data).unwrap();

        assert!(bundle.get_module(0).is_some());
        assert!(bundle.get_module(1).is_none());
        assert!(bundle.get_module(999).is_none());
    }

    #[test]
    fn test_invalid_magic() {
        let mut data = make_test_bundle(&[], "");
        // Corrupt the magic number
        data[0] = 0x00;
        let err = IndexedRamBundle::from_bytes(&data).unwrap_err();
        assert!(matches!(err, RamBundleError::InvalidMagic));
    }

    #[test]
    fn test_too_short_header() {
        let err = IndexedRamBundle::from_bytes(&[0xE5, 0xD1, 0x0B, 0xFB]).unwrap_err();
        assert!(matches!(err, RamBundleError::TooShort));
    }

    #[test]
    fn test_too_short_for_table() {
        // Valid header claiming 1000 modules but no table data
        let mut data = Vec::new();
        data.extend_from_slice(&RAM_BUNDLE_MAGIC.to_le_bytes());
        data.extend_from_slice(&1000_u32.to_le_bytes());
        data.extend_from_slice(&0_u32.to_le_bytes());
        let err = IndexedRamBundle::from_bytes(&data).unwrap_err();
        assert!(matches!(err, RamBundleError::TooShort));
    }

    #[test]
    fn test_module_extends_beyond_data() {
        // Build a bundle but truncate it
        let data = make_test_bundle(&[Some("hello world")], "");
        let truncated = &data[..data.len() - 5];
        let err = IndexedRamBundle::from_bytes(truncated).unwrap_err();
        assert!(matches!(err, RamBundleError::InvalidEntry(_)));
    }

    #[test]
    fn test_module_iteration_order() {
        let modules = vec![Some("mod0"), None, Some("mod2"), None, Some("mod4")];
        let data = make_test_bundle(&modules, "");
        let bundle = IndexedRamBundle::from_bytes(&data).unwrap();

        let ids: Vec<u32> = bundle.modules().map(|m| m.id).collect();
        assert_eq!(ids, vec![0, 2, 4]);
    }

    #[test]
    fn test_is_unbundle_dir_nonexistent() {
        assert!(!is_unbundle_dir(Path::new("/nonexistent/path")));
    }

    #[test]
    fn test_display_errors() {
        assert_eq!(
            RamBundleError::InvalidMagic.to_string(),
            "invalid RAM bundle magic number"
        );
        assert_eq!(
            RamBundleError::TooShort.to_string(),
            "data too short for RAM bundle header"
        );
        assert_eq!(
            RamBundleError::InvalidEntry("bad".to_string()).to_string(),
            "invalid module entry: bad"
        );
    }

    #[test]
    fn test_ram_bundle_type_equality() {
        assert_eq!(RamBundleType::Indexed, RamBundleType::Indexed);
        assert_eq!(RamBundleType::Unbundle, RamBundleType::Unbundle);
        assert_ne!(RamBundleType::Indexed, RamBundleType::Unbundle);
    }
}
