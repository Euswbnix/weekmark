//! The pinned Codex runtime (`data/codex-pin.toml`, design §2.3): its version, the only models a
//! run may name, and one verified asset per target. Nothing outside the pin is ever installed.

use std::cmp::Ordering;
use std::fmt;
use std::sync::LazyLock;

use pagelamp_core::ai::AiFeature;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub version: String,
    /// The GitHub release tag (`rust-v<version>`).
    pub tag: String,
    /// "Use my installed Codex" (D12): versions from `tested_min` up to, not including,
    /// `tested_below`.
    pub tested_min: String,
    pub tested_below: String,
    pub models: PinModels,
    #[serde(rename = "asset")]
    pub assets: Vec<PinAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinModels {
    /// The only models a run may name with `-m`.
    pub supported: Vec<String>,
    pub default: FeatureModels,
    /// Tried once after `model_not_found` for the default.
    pub fallback: FeatureModels,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureModels {
    pub study_plan: String,
    pub weekly_explanation: String,
    pub weekly_note: String,
    pub course_calendar: String,
}

impl FeatureModels {
    pub fn get(&self, feature: AiFeature) -> &str {
        match feature {
            AiFeature::StudyPlan => &self.study_plan,
            AiFeature::WeeklyExplanation => &self.weekly_explanation,
            AiFeature::WeeklyNote => &self.weekly_note,
            AiFeature::CourseCalendar => &self.course_calendar,
        }
    }
}

/// One release asset: a zstd-compressed Codex binary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinAsset {
    /// A Rust target triple (`aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, …).
    pub target: String,
    pub name: String,
    /// Compressed size in bytes (shown before the download).
    pub size: u64,
    /// Lower-case hex SHA-256 of the compressed file (the GitHub asset digest).
    pub sha256: String,
}

impl PinAsset {
    /// The binary's file name once unpacked.
    pub fn binary_name(&self) -> &'static str {
        if self.target.contains("windows") {
            "codex.exe"
        } else {
            "codex"
        }
    }
}

static PIN: LazyLock<Pin> = LazyLock::new(|| {
    toml::from_str(include_str!("../../data/codex-pin.toml")).expect("codex-pin.toml is valid")
});

/// The pin this build of PageLamp carries.
pub fn pin() -> &'static Pin {
    &PIN
}

impl Pin {
    pub fn asset(&self, target: &str) -> Option<&PinAsset> {
        self.assets.iter().find(|asset| asset.target == target)
    }

    /// Where the asset is downloaded from: OpenAI's own GitHub release, never a copy.
    pub fn download_url(&self, asset: &PinAsset) -> String {
        format!(
            "https://github.com/openai/codex/releases/download/{}/{}",
            self.tag, asset.name
        )
    }

    pub fn version(&self) -> Version {
        Version::parse(&self.version).expect("the pinned version parses")
    }

    pub fn is_supported_model(&self, model: &str) -> bool {
        self.models.supported.iter().any(|m| m == model)
    }

    /// Whether an installed Codex may be used instead of the managed one (D12).
    pub fn in_tested_range(&self, version: &Version) -> bool {
        let min = Version::parse(&self.tested_min).expect("tested_min parses");
        let below = Version::parse(&self.tested_below).expect("tested_below parses");
        *version >= min && *version < below
    }
}

/// A Codex version (`0.158.0`, `0.158.0-alpha.2.1`); `codex --version` prints
/// `codex-cli 0.158.0`, so the last word is used. A pre-release sorts before its release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let word = text.split_whitespace().last()?;
        let word = word.strip_prefix('v').unwrap_or(word);
        let (numbers, pre) = match word.split_once('-') {
            Some((numbers, pre)) if !pre.is_empty() => (numbers, Some(pre.to_string())),
            Some(_) => return None,
            None => (word, None),
        };
        let mut parts = numbers.split('.').map(|n| n.parse::<u64>().ok());
        let (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => compare_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Dot-separated identifiers, numbers numerically (semver §11).
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// The Codex target for the **running** OS and CPU, not PageLamp's build target: the x86_64
/// slice of the universal macOS app under Rosetta, or x64 PageLamp on Windows on Arm, gets the
/// native binary (D14). `None` where Codex isn't offered.
pub fn running_target() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        Some(if cfg!(target_arch = "aarch64") || macos_translated() {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        })
    }
    #[cfg(target_os = "windows")]
    {
        windows_native_target()
    }
    #[cfg(target_os = "linux")]
    {
        cfg!(target_arch = "x86_64").then_some("x86_64-unknown-linux-musl")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// Whether this process runs under Rosetta (`sysctl.proc_translated` = 1).
#[cfg(target_os = "macos")]
fn macos_translated() -> bool {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: a documented sysctl name; `value` and `size` describe a valid c_int buffer.
    let status = unsafe {
        libc::sysctlbyname(
            c"sysctl.proc_translated".as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    status == 0 && value == 1
}

/// The machine's native CPU (`IsWow64Process2`), whatever this process was built for.
#[cfg(target_os = "windows")]
fn windows_native_target() -> Option<&'static str> {
    use windows_sys::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    let mut process = 0;
    let mut native = 0;
    // SAFETY: the pseudo-handle of this process and two valid out-parameters.
    let ok = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) };
    let native = if ok != 0 {
        native
    } else if cfg!(target_arch = "aarch64") {
        IMAGE_FILE_MACHINE_ARM64
    } else {
        IMAGE_FILE_MACHINE_AMD64
    };
    match native {
        IMAGE_FILE_MACHINE_ARM64 => Some("aarch64-pc-windows-msvc"),
        IMAGE_FILE_MACHINE_AMD64 => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_parses_and_names_only_supported_models() {
        let pin = pin();
        assert_eq!(pin.tag, format!("rust-v{}", pin.version));
        assert!(pin.version().pre.is_none(), "never an alpha");
        for feature in AiFeature::ALL {
            assert!(pin.is_supported_model(pin.models.default.get(feature)));
            assert!(pin.is_supported_model(pin.models.fallback.get(feature)));
        }
        for asset in &pin.assets {
            assert_eq!(asset.sha256.len(), 64, "{}", asset.name);
            assert!(
                asset
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert!(asset.name.starts_with("codex-") && asset.name.ends_with(".zst"));
            assert!(asset.name.contains(&asset.target), "{}", asset.name);
            assert!(asset.size > 10_000_000);
        }
        // D14: every platform PageLamp ships.
        for target in [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-musl",
        ] {
            assert!(pin.asset(target).is_some(), "{target}");
        }
        assert_eq!(
            pin.asset("x86_64-pc-windows-msvc").unwrap().binary_name(),
            "codex.exe"
        );
        assert!(pin.in_tested_range(&pin.version()));
        assert!(
            pin.download_url(pin.asset("aarch64-apple-darwin").unwrap())
                .starts_with("https://github.com/openai/codex/releases/download/rust-v")
        );
    }

    #[test]
    fn versions_compare_like_semver() {
        let v = |text: &str| Version::parse(text).unwrap();
        assert_eq!(v("codex-cli 0.158.0"), v("0.158.0"));
        assert!(v("0.158.0-alpha.2.1") < v("0.158.0"));
        assert!(v("0.158.0-alpha.2") < v("0.158.0-alpha.10"));
        assert!(v("0.144.1") < v("0.158.0") && v("0.158.1") > v("0.158.0"));
        assert!(v("0.159.0-alpha.1") > v("0.158.9"));
        assert_eq!(v("0.158.0-alpha.2.1").to_string(), "0.158.0-alpha.2.1");
        for bad in ["", "0.158", "0.158.0.1", "x.1.2", "0.158.0-", "codex-cli"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
        let pin = pin();
        assert!(!pin.in_tested_range(&v("0.157.1")));
        assert!(!pin.in_tested_range(&v("0.159.0")));
        assert!(
            !pin.in_tested_range(&v("0.158.0-alpha.2.1")),
            "before the tested release"
        );
    }

    #[test]
    fn this_machine_has_a_target_where_codex_is_offered() {
        if cfg!(any(target_os = "macos", target_os = "windows"))
            || cfg!(all(target_os = "linux", target_arch = "x86_64"))
        {
            let target = running_target().unwrap();
            assert!(pin().asset(target).is_some(), "{target}");
        }
    }
}
