//! Импорт обоев: Steam Workshop (через steamcmd пользователя), прямые .zip / .mp4 / .webm.
use crate::settings::Settings;
use anyhow::{anyhow, bail, Context, Result};
use std::{
    fs, io,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use url::Url;

const WE_APP: &str = "431960"; // Wallpaper Engine
const NO_WINDOW: u32 = 0x0800_0000;

pub fn from_url(input: &str, cfg: &Settings) -> Result<String> {
    let root = PathBuf::from(&cfg.folder);
    fs::create_dir_all(&root)?;
    if let Some(id) = steam_id(input) {
        return steam(&id, cfg, &root);
    }
    let url = Url::parse(input).context("это не ссылка")?;
    let name = url.path_segments().and_then(|s| s.last()).unwrap_or("").to_string();
    let ext = Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "zip" => zip(&url, &name, &root),
        "mp4" | "webm" => video(&url, &name, &ext, &root),
        _ => bail!("Нужна ссылка/ID Steam Workshop или прямая ссылка на .zip, .mp4, .webm"),
    }
}

fn steam_id(s: &str) -> Option<String> {
    let s = s.trim();
    let digits = |v: &str| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit());
    if digits(s) {
        return Some(s.into());
    }
    let u = Url::parse(s).ok()?;
    if !u.host_str()?.ends_with("steamcommunity.com") {
        return None;
    }
    u.query_pairs().find(|(k, _)| k == "id").map(|(_, v)| v.into_owned()).filter(|v| digits(v))
}

fn steam(id: &str, cfg: &Settings, root: &Path) -> Result<String> {
    if cfg.steamcmd.is_empty() || cfg.steam_user.is_empty() {
        bail!("Для Steam Workshop укажи путь к steamcmd и логин Steam в настройках");
    }
    let out = Command::new(&cfg.steamcmd)
        .args(["+login", &cfg.steam_user, "+workshop_download_item", WE_APP, id, "+quit"])
        .stdin(Stdio::null()) // если нужен ввод (Steam Guard), steamcmd завершится, а не зависнет
        .creation_flags(NO_WINDOW)
        .output()
        .context("не удалось запустить steamcmd")?;
    let dir = Path::new(&cfg.steamcmd)
        .parent()
        .unwrap_or(Path::new("."))
        .join("steamapps/workshop/content")
        .join(WE_APP)
        .join(id);
    if !dir.join("project.json").exists() {
        let text = String::from_utf8_lossy(&out.stdout);
        let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        bail!("steamcmd не скачал обои (нужен аккаунт с Wallpaper Engine и сохранённый вход): {last}");
    }
    copy_dir(&dir, &unique(root, id))?;
    Ok(format!("Скачано из Steam Workshop: {id}"))
}

fn zip(url: &Url, name: &str, root: &Path) -> Result<String> {
    let tmp = root.join(format!(".luma-tmp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp)?;
    let file = tmp.join("download.zip");
    download(url, &file)?;
    let unpack = tmp.join("x");
    let mut ar = zip::ZipArchive::new(fs::File::open(&file)?)?;
    for i in 0..ar.len() {
        let mut e = ar.by_index(i)?;
        let Some(rel) = e.enclosed_name() else { continue }; // защита от ../
        let out = unpack.join(rel);
        if e.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(p) = out.parent() {
                fs::create_dir_all(p)?;
            }
            io::copy(&mut e, &mut fs::File::create(&out)?)?;
        }
    }
    let src = find_project(&unpack).ok_or_else(|| anyhow!("в архиве нет project.json"))?;
    let dest = unique(root, &stem(name));
    copy_dir(&src, &dest)?;
    let _ = fs::remove_dir_all(&tmp);
    Ok(format!("Добавлено: {}", dest.file_name().unwrap().to_string_lossy()))
}

fn video(url: &Url, name: &str, ext: &str, root: &Path) -> Result<String> {
    let title = stem(name);
    let dest = unique(root, &title);
    fs::create_dir_all(&dest)?;
    let file = format!("video.{ext}");
    if let Err(e) = download(url, &dest.join(&file)) {
        let _ = fs::remove_dir_all(&dest);
        return Err(e);
    }
    let meta = serde_json::json!({ "title": title, "type": "video", "file": file });
    fs::write(dest.join("project.json"), meta.to_string())?;
    Ok(format!("Добавлено: {title}"))
}

fn download(url: &Url, to: &Path) -> Result<()> {
    let resp = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .build()
        .get(url.as_str())
        .set("User-Agent", "LumaWallpaper/0.3")
        .call()
        .map_err(|e| anyhow!("загрузка не удалась: {e}"))?;
    let mut f = fs::File::create(to)?;
    io::copy(&mut resp.into_reader().take(4 << 30), &mut f)?; // не больше 4 ГБ
    Ok(())
}

fn find_project(dir: &Path) -> Option<PathBuf> {
    if dir.join("project.json").exists() {
        return Some(dir.to_path_buf());
    }
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.join("project.json").exists())
}

fn stem(name: &str) -> String {
    Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or("wallpaper").to_string()
}

fn unique(root: &Path, name: &str) -> PathBuf {
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let base = if clean.is_empty() { "wallpaper".to_string() } else { clean };
    let mut p = root.join(&base);
    let mut i = 2;
    while p.exists() {
        p = root.join(format!("{base}-{i}"));
        i += 1;
    }
    p
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}
