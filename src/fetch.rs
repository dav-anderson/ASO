//! The real release source: releases are listed and downloaded with `curl`, run
//! through the same [`Exec`] as every other program (no HTTP crate).
//!
//! The GitHub API lists a repository's releases (`dav-anderson/<program>`) and
//! their assets. The archive is `<program>-<version>-<os>-<arch>.tar.gz` (`os`
//! is `linux` or `macos`, `arch` is `x86_64` or `aarch64`) and the release
//! carries a `SHA256SUMS` asset. The archive's SHA-256 is checked against it here,
//! before anything is unpacked. A release marked `prerelease` is never offered
//! and never installed, not even when it is pinned.
//!
//! No release exists yet (ROADMAP item 26): until one does, `aso upgrade check`
//! says so and `aso install` uses the file `fsp.bin` or `wallet.bin` names.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::Aso;
use crate::exec::Exec;
use crate::program::Program;
use crate::release::{self, Download, Source};
use crate::state;

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Rel {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// GitHub releases, through `curl`.
pub struct CurlSource<'a> {
    exec: &'a dyn Exec,
    api: String,
    os: &'static str,
    arch: String,
}

impl<'a> CurlSource<'a> {
    /// The source for this machine.
    pub fn new(aso: &Aso, exec: &'a dyn Exec) -> CurlSource<'a> {
        let os = if aso.mac { "macos" } else { "linux" };
        CurlSource::for_machine(aso, exec, os, std::env::consts::ARCH)
    }

    /// The source for a machine of `os` and `arch` (the tests name one).
    pub fn for_machine(
        aso: &Aso,
        exec: &'a dyn Exec,
        os: &'static str,
        arch: &str,
    ) -> CurlSource<'a> {
        CurlSource {
            exec,
            api: aso.text("release.url"),
            os,
            arch: arch.to_string(),
        }
    }

    fn curl(&self, args: &[&str]) -> Result<String, String> {
        let mut all: Vec<String> = ["-fsSL", "--max-time", "900"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        all.extend(args.iter().map(|s| s.to_string()));
        let ran = self.exec.run("curl", &all, &[]).map_err(|e| {
            format!("{e} (aso downloads releases with curl: install curl, or put the build in place and name it with `bin`)")
        })?;
        if ran.ok() {
            Ok(ran.stdout)
        } else {
            Err(format!(
                "curl {}: {}",
                args.last().unwrap_or(&""),
                ran.last_line()
            ))
        }
    }

    fn releases(&self, program: Program) -> Result<Vec<Rel>, String> {
        let url = format!(
            "{}/repos/dav-anderson/{}/releases?per_page=100",
            self.api,
            program.name()
        );
        let body = self.curl(&["-H", "Accept: application/vnd.github+json", &url])?;
        let list: Vec<Rel> = serde_json::from_str(&body)
            .map_err(|e| format!("the release list at {url} is not what GitHub sends: {e}"))?;
        Ok(list.into_iter().filter(|r| !r.draft).collect())
    }

    /// The archive for this machine: its name, where it is, and where the
    /// release's `SHA256SUMS` is.
    fn archive(&self, program: Program, version: &str) -> Result<(String, String, String), String> {
        let list = self.releases(program)?;
        let Some(rel) = list
            .into_iter()
            .find(|r| r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name) == version)
        else {
            return Err(format!("{} has no release {version}", program.name()));
        };
        if rel.prerelease {
            return Err(format!(
                "{} {version} is marked as a pre-release on GitHub; aso installs releases only",
                program.name()
            ));
        }
        let name = format!(
            "{}-{version}-{}-{}.tar.gz",
            program.name(),
            self.os,
            self.arch
        );
        let find = |wanted: &str| {
            rel.assets
                .iter()
                .find(|a| a.name == wanted)
                .map(|a| a.browser_download_url.clone())
        };
        let Some(url) = find(&name) else {
            let have: Vec<&str> = rel.assets.iter().map(|a| a.name.as_str()).collect();
            return Err(format!(
                "release {} of {} has no archive for this machine ({} on {}): expected {name}, it has {}",
                rel.tag_name,
                program.name(),
                self.os,
                self.arch,
                have.join(", ")
            ));
        };
        let Some(sums) = find("SHA256SUMS") else {
            return Err(format!(
                "release {} of {} has no SHA256SUMS",
                rel.tag_name,
                program.name()
            ));
        };
        Ok((name, url, sums))
    }
}

/// The SHA-256 `SHA256SUMS` (`<hex>  <name>` lines) gives for `name`.
fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hex, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hex.to_string())
    })
}

/// The first file called `name` under `dir`.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    None
}

impl Source for CurlSource<'_> {
    fn tags(&self, program: Program) -> Result<Vec<String>, String> {
        if !program.publishes() {
            return Err(format!("{} publishes no releases", program.name()));
        }
        Ok(self
            .releases(program)?
            .into_iter()
            .filter(|r| !r.prerelease)
            .map(|r| r.tag_name)
            .collect())
    }

    fn fetch(&self, program: Program, version: &str, dir: &Path) -> Result<Download, String> {
        let (name, url, sums_url) = self.archive(program, version)?;
        let sums = self.curl(&[&sums_url])?;
        let Some(published) = sum_for(&sums, &name) else {
            return Err(format!("{sums_url} does not list {name}"));
        };
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let archive = dir.join(&name);
        let unpacked = dir.join("unpacked");
        let result = (|| {
            self.curl(&["-o", &archive.display().to_string(), &url])?;
            let have = state::sha256_file(&archive)?;
            if !have.eq_ignore_ascii_case(&published) {
                return Err(format!(
                    "the download of {name} failed its checksum (SHA256SUMS says {published}, the file is {have}): it was deleted and nothing was changed"
                ));
            }
            let args = [
                "-xzf".to_string(),
                archive.display().to_string(),
                "-C".into(),
                unpacked.display().to_string(),
            ];
            fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
            let ran = self.exec.run("tar", &args, &[])?;
            if !ran.ok() {
                return Err(format!("cannot unpack {name}: {}", ran.last_line()));
            }
            let found = find_file(&unpacked, program.name())
                .ok_or_else(|| format!("{name} holds no file called {}", program.name()))?;
            let file = dir.join(program.name());
            fs::rename(&found, &file)
                .map_err(|e| format!("cannot move {}: {e}", found.display()))?;
            release::make_executable(&file)?;
            Ok(Download {
                sha256: state::sha256_file(&file)?,
                file,
            })
        })();
        let _ = fs::remove_file(&archive);
        let _ = fs::remove_dir_all(&unpacked);
        result
    }
}
