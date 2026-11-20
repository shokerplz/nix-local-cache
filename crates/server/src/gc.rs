//! Garbage collection for the flat-file binary cache.
//!
//! Roots are the results of jobs from the last N days, plus the newest result of
//! every target regardless of age. Everything reachable from a root is kept:
//! its runtime closure (narinfo `References`) and, when the job left a manifest,
//! every path that job uploaded (the build-time closure). The rest is deleted.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Local};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What the collector needs to know about one job.
pub struct JobRoots {
    pub id: Uuid,
    /// When the job last ran: a restarted job counts from its restart.
    pub built_at: DateTime<Local>,
    pub target_type: String,
    /// host -> store path of the built result
    pub results: HashMap<String, String>,
}

#[derive(Debug, Default, Serialize)]
pub struct GcReport {
    pub retention_days: u32,
    pub root_jobs: usize,
    pub kept_narinfos: usize,
    pub deleted_narinfos: usize,
    pub kept_nars: usize,
    pub deleted_nars: usize,
    pub kept_bytes: u64,
    pub deleted_bytes: u64,
    /// References of kept paths that have no narinfo in the cache.
    pub dangling_references: usize,
    /// Root jobs without a manifest: only their runtime closure is kept, so
    /// `kept_bytes` is lower than it will be once every root has a manifest.
    pub root_jobs_without_manifest: usize,
    /// Targets built within the retention window whose newest build has no
    /// manifest. Deleting now would drop their build-time dependencies and the
    /// next job would have to realise them again, so the sweep is skipped.
    pub unprotected_targets: Vec<String>,
    /// NARs that should have been deleted but could not be.
    pub failed_nar_deletes: usize,
    pub swept: bool,
}

pub fn manifest_path(manifest_dir: &Path, job_id: Uuid) -> PathBuf {
    manifest_dir.join(format!("{}.paths", job_id))
}

/// Records store paths a job needs in the cache. Called once per built host.
pub fn append_manifest<'a>(
    manifest_dir: &Path,
    job_id: Uuid,
    paths: impl Iterator<Item = &'a String>,
) -> std::io::Result<()> {
    let lines: String = paths
        .filter_map(|p| store_hash(p))
        .flat_map(|hash| [hash, "\n"])
        .collect();
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest_path(manifest_dir, job_id))?
        .write_all(lines.as_bytes())
}

/// The 32-character hash part of a store path or store path basename.
pub fn store_hash(path: &str) -> Option<&str> {
    let hash = path.rsplit('/').next()?.get(..32)?;
    hash.bytes()
        .all(|b| b.is_ascii_alphanumeric())
        .then_some(hash)
}

/// Works out what to delete and, unless `dry_run`, deletes it.
///
/// The caller must make sure nothing uploads to the cache while this runs.
pub fn run(
    cache_dir: &Path,
    manifest_dir: &Path,
    mut jobs: Vec<JobRoots>,
    retention_days: u32,
    now: DateTime<Local>,
    dry_run: bool,
) -> Result<GcReport> {
    let mut report = GcReport {
        retention_days,
        ..Default::default()
    };

    // Roots: every job inside the window, plus the newest build of each target.
    let cutoff = now
        .checked_sub_signed(Duration::days(retention_days.into()))
        .context("retention_days is too large")?;
    jobs.sort_by_key(|job| std::cmp::Reverse(job.built_at));
    let mut seen_targets = HashSet::new();
    let mut root_ids = HashSet::new();
    let mut seeds: HashSet<String> = HashSet::new();
    for job in &jobs {
        let recent = job.built_at >= cutoff;
        let newest_for: Vec<&String> = job
            .results
            .keys()
            .filter(|host| seen_targets.insert((job.target_type.clone(), (*host).clone())))
            .collect();
        if job.results.is_empty() || !recent && newest_for.is_empty() {
            continue;
        }
        root_ids.insert(job.id);
        seeds.extend(
            job.results
                .values()
                .filter_map(|p| store_hash(p))
                .map(String::from),
        );
        match fs::read_to_string(manifest_path(manifest_dir, job.id)) {
            Ok(manifest) => seeds.extend(manifest.lines().filter_map(store_hash).map(String::from)),
            Err(e) if e.kind() != ErrorKind::NotFound => {
                return Err(e).context(format!("Failed to read manifest of job {}", job.id));
            }
            Err(_) => {
                report.root_jobs_without_manifest += 1;
                if recent {
                    report.unprotected_targets.extend(
                        newest_for
                            .iter()
                            .map(|host| format!("{}/{}", job.target_type, host)),
                    );
                }
            }
        }
    }
    report.root_jobs = root_ids.len();
    report.unprotected_targets.sort();

    // Mark: follow References from the roots, reading only narinfos that are kept.
    let mut todo: Vec<String> = seeds.iter().cloned().collect();
    let mut keep: HashSet<String> = HashSet::new();
    let mut keep_nars: HashSet<String> = HashSet::new();
    while let Some(hash) = todo.pop() {
        if keep.contains(&hash) {
            continue;
        }
        let narinfo = match fs::read_to_string(cache_dir.join(format!("{}.narinfo", hash))) {
            Ok(narinfo) => narinfo,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // A manifest may list paths whose upload failed; a missing reference is a real gap.
                if !seeds.contains(&hash) {
                    report.dangling_references += 1;
                }
                continue;
            }
            // Anything else must not be mistaken for "not in the cache".
            Err(e) => return Err(e).context(format!("Failed to read narinfo {}", hash)),
        };
        for line in narinfo.lines() {
            if let Some(url) = line.strip_prefix("URL: ") {
                keep_nars.insert(url.trim().to_string());
            } else if let Some(refs) = line.strip_prefix("References: ") {
                todo.extend(
                    refs.split_whitespace()
                        .filter_map(store_hash)
                        .filter(|h| !keep.contains(*h))
                        .map(String::from),
                );
            }
        }
        keep.insert(hash);
    }
    report.kept_narinfos = keep.len();
    if keep.is_empty() {
        bail!("no build result found in the cache, refusing to delete everything");
    }

    // Sweep list: narinfos that were not marked, then NARs no kept narinfo points to.
    let mut dead_narinfos = Vec::new();
    for entry in fs::read_dir(cache_dir).context("Failed to list cache directory")? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "narinfo")
            && !path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| keep.contains(s))
        {
            dead_narinfos.push(path);
        }
    }
    report.deleted_narinfos = dead_narinfos.len();

    let mut dead_nars = Vec::new();
    for entry in fs::read_dir(cache_dir.join("nar")).context("Failed to list nar directory")? {
        let entry = entry?;
        let size = entry.metadata()?.len();
        if keep_nars.contains(&format!("nar/{}", entry.file_name().to_string_lossy())) {
            report.kept_nars += 1;
            report.kept_bytes += size;
        } else {
            report.deleted_nars += 1;
            report.deleted_bytes += size;
            dead_nars.push(entry.path());
        }
    }

    if dry_run || !report.unprotected_targets.is_empty() {
        return Ok(report);
    }

    // Narinfos go first: a crash in between leaves orphan NARs, which the next
    // run removes, instead of narinfos that point at nothing.
    for path in &dead_narinfos {
        fs::remove_file(path).with_context(|| format!("Failed to delete {}", path.display()))?;
    }
    // One undeletable NAR must not stop the rest from being reclaimed.
    report.failed_nar_deletes = dead_nars
        .iter()
        .filter(|path| fs::remove_file(path).is_err())
        .count();
    if let Ok(entries) = fs::read_dir(manifest_dir) {
        for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
            let is_root = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| Uuid::parse_str(s).ok())
                .is_some_and(|id| root_ids.contains(&id));
            if !is_root && path.extension().is_some_and(|e| e == "paths") {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to delete {}", path.display()))?;
            }
        }
    }
    report.swept = true;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct Cache {
        dir: TempDir,
        now: DateTime<Local>,
    }

    /// 32-character fake store hash, e.g. h("a") -> "aaaa…".
    fn h(c: &str) -> String {
        c.repeat(32)
    }

    impl Cache {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("cache/nar")).unwrap();
            fs::create_dir_all(dir.path().join("manifests")).unwrap();
            fs::write(
                dir.path().join("cache/nix-cache-info"),
                "StoreDir: /nix/store\n",
            )
            .unwrap();
            Self {
                dir,
                now: Local::now(),
            }
        }

        fn cache(&self) -> PathBuf {
            self.dir.path().join("cache")
        }

        fn manifests(&self) -> PathBuf {
            self.dir.path().join("manifests")
        }

        /// Adds a store path with its own NAR (named after `nar`) and references.
        fn add(&self, name: &str, nar: &str, refs: &[&str]) {
            let refs: Vec<String> = refs.iter().map(|r| format!("{}-pkg", h(r))).collect();
            fs::write(
                self.cache().join(format!("{}.narinfo", h(name))),
                format!(
                    "StorePath: /nix/store/{}-pkg\nURL: nar/{}.nar.xz\nCompression: xz\nReferences: {}\n",
                    h(name),
                    nar,
                    refs.join(" ")
                ),
            )
            .unwrap();
            fs::write(self.cache().join(format!("nar/{}.nar.xz", nar)), nar).unwrap();
        }

        fn job(&self, days_ago: i64, host: &str, result: &str) -> JobRoots {
            JobRoots {
                id: Uuid::new_v4(),
                built_at: self.now - Duration::days(days_ago),
                target_type: "Nixos".to_string(),
                results: HashMap::from([(
                    host.to_string(),
                    format!("/nix/store/{}-pkg", h(result)),
                )]),
            }
        }

        fn manifest(&self, job: &JobRoots, paths: &[&str]) {
            let lines: Vec<String> = paths.iter().map(|p| h(p)).collect();
            fs::write(manifest_path(&self.manifests(), job.id), lines.join("\n")).unwrap();
        }

        fn run(&self, jobs: Vec<JobRoots>, dry_run: bool) -> Result<GcReport> {
            run(
                &self.cache(),
                &self.manifests(),
                jobs,
                30,
                self.now,
                dry_run,
            )
        }

        fn has(&self, name: &str) -> bool {
            self.cache().join(format!("{}.narinfo", h(name))).exists()
        }

        fn has_nar(&self, nar: &str) -> bool {
            self.cache().join(format!("nar/{}.nar.xz", nar)).exists()
        }

        /// Every narinfo left must have its NAR and every reference it names.
        fn assert_consistent(&self) {
            for entry in fs::read_dir(self.cache()).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "narinfo") {
                    let text = fs::read_to_string(&path).unwrap();
                    for line in text.lines() {
                        if let Some(url) = line.strip_prefix("URL: ") {
                            assert!(self.cache().join(url).exists(), "{url} missing");
                        } else if let Some(refs) = line.strip_prefix("References: ") {
                            for r in refs.split_whitespace() {
                                let hash = store_hash(r).unwrap();
                                assert!(
                                    self.cache().join(format!("{hash}.narinfo")).exists(),
                                    "reference {hash} missing"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn clone_jobs(jobs: &[JobRoots]) -> Vec<JobRoots> {
        jobs.iter()
            .map(|j| JobRoots {
                id: j.id,
                built_at: j.built_at,
                target_type: j.target_type.clone(),
                results: j.results.clone(),
            })
            .collect()
    }

    #[test]
    fn keeps_runtime_closure_and_deletes_the_rest() {
        let c = Cache::new();
        c.add("a", "nar-a", &["b", "a"]); // result, references a dependency and itself
        c.add("b", "nar-b", &[]);
        c.add("c", "nar-c", &["b"]); // old build, not a root
        c.add("d", "nar-b", &[]); // garbage that shares a NAR with a kept path
        let jobs = vec![c.job(1, "host", "a")];
        c.manifest(&jobs[0], &[]);

        let report = c.run(clone_jobs(&jobs), false).unwrap();

        assert!(report.swept);
        assert_eq!((report.kept_narinfos, report.deleted_narinfos), (2, 2));
        assert_eq!((report.kept_nars, report.deleted_nars), (2, 1));
        assert_eq!(report.deleted_bytes, "nar-c".len() as u64);
        assert!(c.has("a") && c.has("b") && !c.has("c") && !c.has("d"));
        assert!(c.has_nar("nar-a") && c.has_nar("nar-b") && !c.has_nar("nar-c"));
        assert!(c.cache().join("nix-cache-info").exists());
        c.assert_consistent();

        let again = c.run(jobs, false).unwrap();
        assert_eq!((again.deleted_narinfos, again.deleted_nars), (0, 0));
    }

    #[test]
    fn dry_run_deletes_nothing() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]);
        c.add("c", "nar-c", &[]);
        let jobs = vec![c.job(1, "host", "a")];
        c.manifest(&jobs[0], &[]);

        let report = c.run(jobs, true).unwrap();

        assert!(!report.swept);
        assert_eq!(report.deleted_narinfos, 1);
        assert!(c.has("c") && c.has_nar("nar-c"));
    }

    #[test]
    fn old_builds_expire_but_the_newest_of_each_target_stays() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]); // host1, 40 days ago, superseded
        c.add("b", "nar-b", &[]); // host1, yesterday
        c.add("c", "nar-c", &[]); // host2, 40 days ago, never rebuilt
        let jobs = vec![
            c.job(40, "host1", "a"),
            c.job(1, "host1", "b"),
            c.job(40, "host2", "c"),
        ];
        c.manifest(&jobs[0], &[]);
        c.manifest(&jobs[1], &[]);
        let expired_manifest = manifest_path(&c.manifests(), jobs[0].id);

        let report = c.run(jobs, false).unwrap();

        assert_eq!(report.root_jobs, 2);
        assert!(!c.has("a") && c.has("b") && c.has("c"));
        assert!(!expired_manifest.exists());
    }

    #[test]
    fn manifest_keeps_build_time_paths() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]);
        c.add("b", "nar-b", &["c"]); // compiler: not referenced by the result
        c.add("c", "nar-c", &[]);
        c.add("d", "nar-d", &[]);
        let jobs = vec![c.job(1, "host", "a")];
        c.manifest(&jobs[0], &["a", "b", "z"]); // "z" was never uploaded

        let report = c.run(jobs, false).unwrap();

        assert_eq!(report.dangling_references, 0);
        assert!(c.has("a") && c.has("b") && c.has("c") && !c.has("d"));
        c.assert_consistent();
    }

    #[test]
    fn appended_manifests_are_read_back() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]);
        c.add("b", "nar-b", &[]);
        c.add("c", "nar-c", &[]);
        c.add("d", "nar-d", &[]);
        let jobs = vec![c.job(1, "host", "a")];
        // One call per host of the job, full store paths as the uploader has them.
        let store = |name: &str| format!("/nix/store/{}-pkg", h(name));
        append_manifest(&c.manifests(), jobs[0].id, [store("b")].iter()).unwrap();
        append_manifest(
            &c.manifests(),
            jobs[0].id,
            [store("c"), "junk".into()].iter(),
        )
        .unwrap();

        c.run(jobs, false).unwrap();

        assert!(c.has("a") && c.has("b") && c.has("c") && !c.has("d"));
    }

    #[test]
    fn absurd_retention_is_an_error_not_a_panic() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]);
        let jobs = vec![c.job(1, "host", "a")];
        assert!(run(&c.cache(), &c.manifests(), jobs, u32::MAX, c.now, true).is_err());
    }

    #[test]
    fn sweep_is_skipped_while_a_current_target_has_no_manifest() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]);
        c.add("b", "nar-b", &[]);
        c.add("c", "nar-c", &[]);
        // host1 was built yesterday without a manifest; host2's manifest-less
        // build is outside the window and must not block anything.
        let jobs = vec![c.job(1, "host1", "a"), c.job(40, "host2", "b")];

        let report = c.run(clone_jobs(&jobs), false).unwrap();

        assert!(!report.swept);
        assert_eq!(report.unprotected_targets, vec!["Nixos/host1"]);
        assert_eq!(report.root_jobs_without_manifest, 2);
        assert_eq!(report.deleted_narinfos, 1);
        assert!(c.has("c"));

        c.manifest(&jobs[0], &[]);
        let report = c.run(jobs, false).unwrap();
        assert!(report.swept && report.unprotected_targets.is_empty());
        assert!(c.has("a") && c.has("b") && !c.has("c"));
    }

    #[test]
    fn refuses_to_delete_everything_when_no_root_is_in_the_cache() {
        let c = Cache::new();
        c.add("a", "nar-a", &[]);

        assert!(c.run(vec![], false).is_err());
        assert!(c.run(vec![c.job(1, "host", "z")], false).is_err());
        assert!(c.has("a") && c.has_nar("nar-a"));
    }

    #[test]
    fn store_hash_rejects_anything_that_is_not_a_hash() {
        assert_eq!(
            store_hash(&format!("/nix/store/{}-x", h("a"))),
            Some(h("a").as_str())
        );
        assert_eq!(store_hash(&h("a")), Some(h("a").as_str()));
        assert_eq!(store_hash("short"), None);
        assert_eq!(
            store_hash(&format!("../{}", "a".repeat(40))),
            Some(h("a").as_str())
        );
        assert_eq!(
            store_hash(&format!("{}..{}", "a".repeat(20), "a".repeat(20))),
            None
        );
    }
}
