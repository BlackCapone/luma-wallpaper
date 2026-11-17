//! Обои в формате Wallpaper Engine: project.json, type "video" | "web".
use serde_json::{json, Map, Value};
use std::{fs, path::Path};
use url::Url;

pub struct Loaded {
    pub url: String,
    pub init_js: String,
}

fn read_meta(dir: &Path) -> anyhow::Result<Value> {
    let txt = fs::read_to_string(dir.join("project.json"))?;
    // у project.json из мастерской часто есть BOM
    Ok(serde_json::from_str(txt.trim_start_matches('\u{feff}'))?)
}

pub fn load(dir: &Path) -> anyhow::Result<Loaded> {
    let dir = dunce::canonicalize(dir)?;
    let meta = read_meta(&dir)?;
    let file = meta["file"].as_str().ok_or_else(|| anyhow::anyhow!("в project.json нет поля file"))?;
    let kind = meta["type"].as_str().unwrap_or("").to_lowercase();
    let target_url = Url::from_file_path(dir.join(file)).map_err(|_| anyhow::anyhow!("плохой путь"))?;

    let url = match kind.as_str() {
        "web" => target_url.to_string(),
        "video" => {
            let html = format!(
                "<body style='margin:0;background:#000'><video src='{target_url}' autoplay loop muted playsinline \
                 style='width:100vw;height:100vh;object-fit:cover'></video></body>"
            );
            let page = std::env::temp_dir().join(format!("luma_video_{}.html", std::process::id()));
            fs::write(&page, html)?;
            Url::from_file_path(&page).unwrap().to_string()
        }
        other => anyhow::bail!("тип «{other}» пока не поддерживается"),
    };

    let mut props = Map::new();
    if let Some(p) = meta["general"]["properties"].as_object() {
        for (k, v) in p {
            props.insert(k.clone(), json!({ "value": v["value"] }));
        }
    }
    let init_js = format!(
        r#"
        window.wallpaperRegisterAudioListener = function (cb) {{ window.__lumaAudio = cb; }};
        window.__lumaPause = function (p) {{
          document.querySelectorAll('video,audio').forEach(function (m) {{ if (p) m.pause(); else m.play().catch(function () {{}}); }});
          var l = window.wallpaperPropertyListener;
          if (l && l.setPaused) l.setPaused(p);
        }};
        window.addEventListener('load', function () {{
          setTimeout(function () {{
            var l = window.wallpaperPropertyListener;
            if (l && l.applyUserProperties) l.applyUserProperties({props});
          }}, 0);
        }});
        "#,
        props = Value::Object(props)
    );
    Ok(Loaded { url, init_js })
}

/// Обходит подпапки root, берёт те, где есть project.json.
pub fn scan(root: &Path) -> Vec<Value> {
    let mut out = vec![];
    let Ok(rd) = fs::read_dir(root) else { return out };
    for e in rd.flatten() {
        let dir = e.path();
        let Ok(m) = read_meta(&dir) else { continue };
        let kind = m["type"].as_str().unwrap_or("").to_lowercase();
        let preview = m["preview"]
            .as_str()
            .and_then(|p| Url::from_file_path(dir.join(p)).ok())
            .map(|u| u.to_string());
        out.push(json!({
            "path": dir.to_string_lossy(),
            "title": m["title"].as_str().unwrap_or("Без названия"),
            "type": kind,
            "supported": kind == "video" || kind == "web",
            "preview": preview,
        }));
    }
    out
}
