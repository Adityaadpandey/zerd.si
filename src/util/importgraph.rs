//! Who imports what, read straight from the working tree: no snapshot to go stale, no external tool. Resolves relative
//! imports, tsconfig/jsconfig `paths` aliases ("@/components/Nav" -> src/components/Nav.tsx) and Python imports.
use crate::util::paths::{normalize, resolve};
use crate::util::pyscan;
use crate::util::re::{lit, Re};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const SOURCE: &[&str] = &["py", "ts", "tsx", "js", "jsx", "mjs", "cjs"];
const JS_EXT: &[&str] = &["", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json", "/index.ts", "/index.tsx", "/index.js", "/index.jsx"];
const MAX_FILES: usize = 8000;

fn js_spec() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit(r#"(?:\bfrom|\bimport|\brequire\(|\bimport\()\s*['"]([^'"\n]+)['"]"#))
}
fn py_dynamic() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit(r#"(?:import_module|__import__)\(\s*['"]([\w.]+)['"]"#))
}
pub fn test_hint() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit(r"(^|/)(__tests__|tests?|e2e|spec)(/|$)|\.(test|spec)\.[jt]sx?$|(^|/)test_[^/]+\.py$|_test\.py$"))
}

/// Every source file git knows about (tracked or untracked and not ignored).
pub fn tracked(cwd: &Path) -> Vec<String> {
    let Ok(o) = std::process::Command::new("git").args(["ls-files", "--cached", "--others", "--exclude-standard"]).current_dir(cwd).output() else { return vec![] };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|f| Path::new(f).extension().map_or(false, |e| SOURCE.contains(&e.to_string_lossy().as_ref())))
        .take(MAX_FILES)
        .map(String::from)
        .collect()
}

/// tsconfig allows comments and trailing commas; strip them before parsing.
fn jsonc(path: &Path) -> Value {
    let Ok(text) = std::fs::read_to_string(path) else { return Value::Null };
    let mut out = String::with_capacity(text.len());
    let b: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '"' => {
                out.push('"');
                i += 1;
                while i < b.len() {
                    out.push(b[i]);
                    if b[i] == '\\' && i + 1 < b.len() {
                        out.push(b[i + 1]);
                        i += 2;
                        continue;
                    }
                    if b[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '/' if b.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    let out = lit(r",(\s*[}\]])").replace_all(&out, "$1").to_string();
    serde_json::from_str(&out).unwrap_or(Value::Null)
}

/// compilerOptions of a tsconfig with its `extends` chain folded in (the child wins, like tsc), plus the folder that
/// `paths` is relative to when no baseUrl is set.
fn compiler_options(path: &Path, seen: &mut HashSet<PathBuf>) -> (serde_json::Map<String, Value>, PathBuf) {
    let path = resolve(path);
    if !seen.insert(path.clone()) {
        return (Default::default(), path.parent().unwrap_or(Path::new(".")).to_path_buf());
    }
    let conf = jsonc(&path);
    let own = conf["compilerOptions"].as_object().cloned().unwrap_or_default();
    let mut opts = serde_json::Map::new();
    let mut paths_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let parents: Vec<String> = match &conf["extends"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => vec![],
    };
    for r in parents {
        let Some(base) = find_extended(path.parent().unwrap_or(Path::new(".")), &r) else { continue };
        let (inherited, inherited_dir) = compiler_options(&base, seen);
        if inherited.contains_key("paths") && !own.contains_key("paths") {
            paths_dir = inherited_dir;
        }
        let bu = inherited.get("baseUrl").cloned();
        opts.extend(inherited);
        if let Some(Value::String(b)) = bu {
            if !own.contains_key("baseUrl") {
                opts.insert("baseUrl".into(), Value::String(resolve(&base.parent().unwrap_or(Path::new(".")).join(b)).to_string_lossy().to_string()));
            }
        }
    }
    opts.extend(own.clone());
    if own.contains_key("paths") {
        paths_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    }
    (opts, paths_dir)
}

fn find_extended(folder: &Path, r: &str) -> Option<PathBuf> {
    let cands: Vec<PathBuf> = if r.starts_with('.') {
        vec![folder.join(r)]
    } else {
        folder.ancestors().map(|d| d.join("node_modules").join(r)).collect()
    };
    for c in cands {
        for form in [c.clone(), c.with_extension("json"), c.join("tsconfig.json")] {
            if form.is_file() {
                return Some(form);
            }
        }
    }
    None
}

type Rule = (String, Vec<String>);

/// `paths` entries of the nearest tsconfig.json/jsconfig.json above a file, as (prefix, [targets relative to the repo]).
struct Aliases {
    rules: HashMap<String, Vec<Rule>>,
    /// Monorepo packages by their package.json name: ("@plane/utils", "packages/utils"), longest name first.
    packages: Vec<(String, String)>,
}

fn rel_to(cwd: &Path, p: &Path) -> String {
    match p.strip_prefix(resolve(cwd)) {
        Ok(r) => {
            let s = r.to_string_lossy().replace('\\', "/");
            if s == "." { String::new() } else { s }
        }
        Err(_) => p.to_string_lossy().to_string(),
    }
}

fn alias_target(cwd: &Path, base: &Path, t: &str) -> String {
    // "src/*" -> "src/*" relative to the repo; a pattern without * stays a plain path
    // normalized, or a tsconfig in apps/web with "./src/*" gives "apps/web/./src/*", which matches no real file
    let head = rel_to(cwd, &normalize(&base.join(t.split('*').next().unwrap_or(""))));
    if t.contains('*') && !head.is_empty() { format!("{head}/*").trim_start_matches('/').to_string() } else if t.contains('*') { "*".into() } else { head }
}

impl Aliases {
    fn new(cwd: &Path) -> Aliases {
        let mut rules = HashMap::new();
        for cfg in ["tsconfig.json", "jsconfig.json"] {
            let Ok(o) = std::process::Command::new("git").args(["ls-files", "--cached", "--others", "--exclude-standard", &format!("*{cfg}")]).current_dir(cwd).output() else { continue };
            for p in String::from_utf8_lossy(&o.stdout).lines().filter(|p| Path::new(p).file_name().map_or(false, |n| n == cfg)) {
                let found = cwd.join(p);
                let (conf, paths_dir) = compiler_options(&found, &mut HashSet::new());
                let base = match conf.get("baseUrl").and_then(Value::as_str) {
                    Some(b) => resolve(&found.parent().unwrap_or(Path::new(".")).join(b)),
                    None => resolve(&paths_dir),
                };
                let mut rs: Vec<Rule> = vec![];
                if let Some(paths) = conf.get("paths").and_then(Value::as_object) {
                    for (pat, targets) in paths {
                        let prefix = pat.split('*').next().unwrap_or("").to_string();
                        let ts = targets.as_array().map(|a| a.iter().filter_map(|t| t.as_str()).map(|t| alias_target(cwd, &base, t)).collect()).unwrap_or_default();
                        rs.push((prefix, ts));
                    }
                }
                if conf.contains_key("baseUrl") {
                    rs.push((String::new(), vec![alias_target(cwd, &base, "*")])); // bare imports resolve against baseUrl
                }
                rules.insert(rel_to(cwd, &resolve(found.parent().unwrap_or(Path::new(".")))), rs);
            }
        }
        Aliases { rules, packages: workspace_packages(cwd) }
    }

    /// "@plane/utils/datetime" -> packages/utils/src/datetime.ts. The package.json usually points at a build folder
    /// (dist/) that git doesn't track, so the source is looked for in src/ first, then at the package's top.
    fn package_file(&self, spec: &str, known: &HashSet<String>) -> Option<String> {
        let (name, dir) = self.packages.iter().find(|(n, _)| spec == n || spec.strip_prefix(n.as_str()).map_or(false, |r| r.starts_with('/')))?;
        let sub = spec[name.len()..].trim_start_matches('/');
        let sub = if sub.is_empty() { "index" } else { sub };
        [format!("{dir}/src/{sub}"), format!("{dir}/{sub}")].iter().find_map(|base| resolve_js(base, known))
    }

    fn for_file(&self, rel: &str) -> &[Rule] {
        let mut d = Path::new(rel).parent();
        while let Some(dir) = d {
            let key = dir.to_string_lossy().to_string();
            if let Some(r) = self.rules.get(&key) {
                return r;
            }
            if key.is_empty() {
                break;
            }
            d = dir.parent();
        }
        &[]
    }
}

/// Every package.json in the repo that has a name, as (name, folder). Folders git ignores (node_modules) aren't listed.
fn workspace_packages(cwd: &Path) -> Vec<(String, String)> {
    let Ok(o) = std::process::Command::new("git").args(["ls-files", "--cached", "--others", "--exclude-standard", "*package.json"]).current_dir(cwd).output() else { return vec![] };
    let mut found: Vec<(String, String)> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|p| *p == "package.json" || p.ends_with("/package.json"))
        .filter_map(|p| {
            let name = jsonc(&cwd.join(p)).get("name")?.as_str()?.to_string();
            Some((name, p.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string())))
        })
        .collect();
    found.sort_by_key(|(n, _)| std::cmp::Reverse(n.len())); // "@a/ui-kit" before "@a/ui"
    found
}

fn resolve_js(base: &str, known: &HashSet<String>) -> Option<String> {
    let base = base.trim_end_matches('/');
    JS_EXT.iter().map(|e| format!("{base}{e}")).find(|c| known.contains(c))
}

/// Folders Python imports are written from. A repo often keeps its package below the top: `apps/api/plane/...` is
/// imported as `plane...`, so `apps/api` is a root. A folder is one when it holds a `manage.py`, or a package (a folder
/// with `__init__.py`) whose own folder isn't a package. The repo top is always one.
fn py_roots(known: &HashSet<String>) -> Vec<String> {
    let parent = |p: &str| p.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string());
    let is_pkg = |d: &str| !d.is_empty() && known.contains(&format!("{d}/__init__.py"));
    let mut roots: BTreeSet<String> = BTreeSet::from([String::new()]);
    for f in known {
        if f == "manage.py" || f.ends_with("/manage.py") {
            roots.insert(parent(f));
        } else if f.ends_with("__init__.py") {
            let pkg = parent(f);
            if !is_pkg(&parent(&pkg)) {
                roots.insert(parent(&pkg));
            }
        }
    }
    roots.into_iter().collect()
}

fn py_targets(rel: &str, src: &str, known: &HashSet<String>, roots: &[String]) -> HashSet<String> {
    // only the roots this file sits under, innermost first, so two projects with a `utils` package don't mix
    let mut mine: Vec<&String> = roots.iter().filter(|r| r.is_empty() || rel.starts_with(&format!("{r}/"))).collect();
    mine.sort_by_key(|r| std::cmp::Reverse(r.len()));
    let mut names: Vec<String> = vec![];
    for im in pyscan::imports(src) {
        if im.level > 0 {
            let mut pkg = Path::new(rel).parent().unwrap_or(Path::new("")).to_path_buf();
            for _ in 0..im.level - 1 {
                pkg = pkg.parent().unwrap_or(Path::new("")).to_path_buf();
            }
            let modpath = if im.module.is_empty() { pkg.to_string_lossy().to_string() } else { pkg.join(im.module.replace('.', "/")).to_string_lossy().to_string() };
            for n in &im.names {
                names.push(format!("{modpath}/{n}").trim_start_matches(['.', '/']).replace('/', "."));
            }
            names.push(modpath.replace('/', "."));
        } else if !im.module.is_empty() {
            names.push(im.module.clone());
            for n in &im.names {
                names.push(format!("{}.{n}", im.module)); // `from pkg import mod` imports pkg/mod.py
            }
        }
    }
    let mut at = 0;
    while let Some(c) = py_dynamic().captures(&src[at..]) {
        let (_, e) = py_dynamic().find(&src[at..]).unwrap();
        names.extend(c[1].clone());
        at += e.max(1);
    }
    let mut found = HashSet::new();
    for name in names {
        let parts: Vec<&str> = name.split('.').collect();
        // an absolute import may start at any root this file is under (a relative one is already spelled from the top,
        // so the inner roots just miss and the top finds it)
        'roots: for root in &mine {
            for i in (1..=parts.len()).rev() {
                let base = if root.is_empty() { parts[..i].join("/") } else { format!("{root}/{}", parts[..i].join("/")) };
                if let Some(hit) = [format!("{base}.py"), format!("{base}/__init__.py")].into_iter().find(|c| known.contains(c)) {
                    found.insert(hit);
                    break 'roots;
                }
            }
        }
    }
    found
}

fn js_targets(rel: &str, src: &str, known: &HashSet<String>, aliases: &Aliases) -> HashSet<String> {
    let mut specs: BTreeSet<String> = BTreeSet::new();
    let mut at = 0;
    while at < src.len() {
        let Some(c) = js_spec().captures(&src[at..]) else { break };
        let (_, e) = js_spec().find(&src[at..]).unwrap();
        specs.extend(c[1].clone());
        at += e.max(1);
    }
    let mut found = HashSet::new();
    for spec in specs {
        let mut hit = None;
        if spec.starts_with('.') {
            let joined = normalize(&Path::new(rel).parent().unwrap_or(Path::new("")).join(&spec));
            hit = resolve_js(&joined.to_string_lossy().replace('\\', "/"), known);
        } else {
            'rules: for (prefix, targets) in aliases.for_file(rel) {
                if !prefix.is_empty() && !spec.starts_with(prefix.as_str()) {
                    continue;
                }
                let rest = &spec[prefix.len()..];
                for t in targets {
                    let cand = if t.contains('*') { t.replace('*', rest) } else { t.clone() };
                    hit = resolve_js(&cand, known);
                    if hit.is_some() {
                        break 'rules;
                    }
                }
            }
            if hit.is_none() {
                hit = aliases.package_file(&spec, known);
            }
        }
        found.extend(hit);
    }
    found
}

pub struct ImportGraph {
    pub files: Vec<String>,
    pub out: HashMap<String, HashSet<String>>,
    pub inc: HashMap<String, HashSet<String>>,
}

impl ImportGraph {
    fn build(cwd: &Path, files: Vec<String>) -> ImportGraph {
        let known: HashSet<String> = files.iter().cloned().collect();
        let aliases = Aliases::new(cwd);
        let roots = py_roots(&known);
        let mut out: HashMap<String, HashSet<String>> = files.iter().map(|f| (f.clone(), HashSet::new())).collect();
        let mut inc: HashMap<String, HashSet<String>> = HashMap::new();
        for f in &files {
            let Ok(src) = std::fs::read_to_string(cwd.join(f)) else { continue };
            let mut targets = if f.ends_with(".py") { py_targets(f, &src, &known, &roots) } else { js_targets(f, &src, &known, &aliases) };
            targets.remove(f);
            for t in &targets {
                inc.entry(t.clone()).or_default().insert(f.clone());
            }
            out.insert(f.clone(), targets);
        }
        ImportGraph { files, out, inc }
    }

    /// A path, a path suffix or a bare file name -> the one file it means (or the candidates when it is ambiguous).
    pub fn find_file(&self, name: &str) -> (Option<String>, Vec<String>) {
        let name = name.trim().trim_start_matches("./");
        if self.out.contains_key(name) {
            return (Some(name.to_string()), vec![]);
        }
        let mut hits: Vec<String> = self.files.iter().filter(|f| f.ends_with(&format!("/{name}")) || *f == name).cloned().collect();
        if hits.is_empty() {
            let stem = Path::new(name).file_stem();
            hits = self.files.iter().filter(|f| Path::new(f).file_stem() == stem).cloned().collect();
        }
        if hits.len() == 1 { (Some(hits.remove(0)), vec![]) } else { (None, hits.into_iter().take(8).collect()) }
    }

    /// Files that depend on `rel` directly or through others, nearest first.
    pub fn blast_radius(&self, rel: &str, depth: usize, limit: usize) -> Vec<(String, usize)> {
        let mut seen: HashMap<String, usize> = HashMap::from([(rel.to_string(), 0)]);
        let mut queue = VecDeque::from([rel.to_string()]);
        let mut out = vec![];
        while let Some(cur) = queue.pop_front() {
            if seen[&cur] >= depth {
                continue;
            }
            let mut deps: Vec<&String> = self.inc.get(&cur).map(|s| s.iter().collect()).unwrap_or_default();
            deps.sort();
            for d in deps {
                if !seen.contains_key(d) {
                    let h = seen[&cur] + 1;
                    seen.insert(d.clone(), h);
                    out.push((d.clone(), h));
                    queue.push_back(d.clone());
                }
            }
        }
        out.truncate(limit);
        out
    }

    pub fn tests_for(&self, rel: &str) -> Vec<String> {
        let stem = Path::new(rel).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut set: BTreeSet<String> = BTreeSet::new();
        for f in &self.files {
            if !test_hint().is_match(f) {
                continue;
            }
            let fs = Path::new(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let bare = fs.replace(".test", "").replace(".spec", "");
            if bare == stem || bare == format!("test_{stem}") || fs.contains(&stem) {
                set.insert(f.clone());
            }
        }
        for (f, _) in self.blast_radius(rel, 2, 200) {
            if test_hint().is_match(&f) {
                set.insert(f);
            }
        }
        set.into_iter().take(20).collect()
    }
}

fn mtime(p: &Path) -> f64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64())
}

type CacheEntry = ((usize, u64), Arc<ImportGraph>);

/// Built once per working-tree state: rebuilt when the set of files or the newest modification time changes.
pub fn load(cwd: &Path) -> Arc<ImportGraph> {
    static C: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    let cwd = resolve(cwd);
    let files = tracked(&cwd);
    let newest = files.iter().map(|f| mtime(&cwd.join(f))).fold(0.0, f64::max);
    let stamp = (files.len(), newest.to_bits());
    let cache = C.get_or_init(|| Mutex::new(HashMap::new()));
    let key = cwd.to_string_lossy().to_string();
    if let Some((s, g)) = cache.lock().unwrap().get(&key) {
        if *s == stamp {
            return g.clone();
        }
    }
    let g = Arc::new(ImportGraph::build(&cwd, files));
    cache.lock().unwrap().insert(key, (stamp, g.clone()));
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn js_relative_and_py() {
        let known: HashSet<String> = ["a/b.ts", "a/c/index.ts", "pkg/mod.py", "pkg/__init__.py"].iter().map(|s| s.to_string()).collect();
        let al = Aliases { rules: HashMap::new(), packages: vec![] };
        let t = js_targets("a/x.ts", "import b from './b';\nimport c from \"./c\";", &known, &al);
        assert!(t.contains("a/b.ts") && t.contains("a/c/index.ts"));
        let p = py_targets("main.py", "from pkg import mod\n", &known, &py_roots(&known));
        assert!(p.contains("pkg/mod.py"));
    }

    #[test]
    fn workspace_packages_resolve_to_their_source() {
        // a monorepo imports "@plane/utils", whose package.json points at dist/; the code we can read is in src/
        let known: HashSet<String> = ["packages/utils/src/index.ts", "packages/utils/src/datetime.ts", "packages/ui/index.tsx", "apps/web/page.tsx"].iter().map(|s| s.to_string()).collect();
        let al = Aliases { rules: HashMap::new(), packages: vec![("@plane/utils".into(), "packages/utils".into()), ("@plane/ui".into(), "packages/ui".into())] };
        let t = js_targets("apps/web/page.tsx", "import { a } from \"@plane/utils\";\nimport { d } from '@plane/utils/datetime';\nimport { B } from '@plane/ui';\nimport x from '@plane/utilsx';", &known, &al);
        assert_eq!(t, HashSet::from(["packages/utils/src/index.ts", "packages/utils/src/datetime.ts", "packages/ui/index.tsx"].map(String::from)));
    }

    #[test]
    fn py_imports_resolve_from_the_project_folder_not_only_the_repo_root() {
        // a Django app under apps/api/ imports `plane.utils.x`, which lives at apps/api/plane/utils/x.py
        let known: HashSet<String> = ["apps/api/manage.py", "apps/api/plane/__init__.py", "apps/api/plane/utils/__init__.py", "apps/api/plane/utils/log.py", "apps/api/plane/views/base.py", "apps/api/plane/views/__init__.py", "tools/plane/utils/log.py"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let roots = py_roots(&known);
        assert!(roots.contains(&"apps/api".to_string()));
        let p = py_targets("apps/api/plane/views/base.py", "from plane.utils.log import log_exception\n", &known, &roots);
        assert_eq!(p, HashSet::from(["apps/api/plane/utils/log.py".to_string()]), "the file's own project, not tools/");
    }
}
