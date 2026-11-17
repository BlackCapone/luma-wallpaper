//! Текстуры .tex Wallpaper Engine -> RGBA8 (берётся самый крупный mip-уровень).
//! Структура: "TEXV…\0" "TEXI…\0", заголовок (format, flags, размеры текстуры и картинки),
//! контейнер "TEXB000N\0", число изображений, [image_format (N>=3)], [mp4-флаг (N>=4)],
//! число mip-уровней, затем уровни: w, h, [lz4-флаг и размер после распаковки (N>=2)], размер, данные.
//! Если image_format != -1, данные — готовый PNG/JPEG, иначе пиксели по header.format.
//! ВНИМАНИЕ: ветка TEXB0004 написана по памяти и на реальных файлах не проверялась.
use anyhow::{anyhow, bail, Result};

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .p
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or_else(|| anyhow!("неожиданный конец файла"))?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// Строка до нулевого байта (он поглощается), не длиннее max.
    fn nstr(&mut self, max: usize) -> Result<String> {
        let start = self.p;
        while self.p < self.b.len() && self.b[self.p] != 0 && self.p - start < max {
            self.p += 1;
        }
        let s = String::from_utf8_lossy(&self.b[start..self.p]).into_owned();
        if self.p < self.b.len() && self.b[self.p] == 0 {
            self.p += 1;
        }
        Ok(s)
    }
}

const MAX_SIDE: usize = 16384;

pub fn decode(data: &[u8]) -> Result<Image> {
    let mut r = R { b: data, p: 0 };
    if !r.nstr(16)?.starts_with("TEXV") {
        bail!("это не .tex");
    }
    if !r.nstr(16)?.starts_with("TEXI") {
        bail!("неизвестный заголовок .tex");
    }
    let format = r.i32()?;
    let _flags = r.i32()?;
    let (_tw, _th) = (r.i32()?, r.i32()?);
    let (iw, ih) = (r.i32()?, r.i32()?);
    let _unk = r.i32()?;

    let magic = r.nstr(16)?;
    let ver: u32 = magic
        .strip_prefix("TEXB")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| anyhow!("нет контейнера TEXB (получено {magic:?})"))?;
    let images = r.i32()?;
    let mut image_format = -1;
    let mut mp4 = false;
    if ver >= 3 {
        image_format = r.i32()?;
    }
    if ver >= 4 {
        mp4 = r.i32()? == 1;
    }
    if mp4 {
        bail!("видео-текстуры (mp4) не поддерживаются");
    }
    if images < 1 {
        bail!("в .tex нет изображений");
    }
    let _mips = r.i32()?;
    if ver >= 4 {
        // по памяти: два параметра, строка-условие и ещё один параметр
        r.i32()?;
        r.i32()?;
        r.nstr(4096)?;
        r.i32()?;
    }
    let (w, h) = (r.i32()?, r.i32()?);
    let (lz4, unpacked) = if ver >= 2 { (r.i32()? != 0, r.i32()? as usize) } else { (false, 0) };
    let n = r.i32()?;
    if n < 0 {
        bail!("отрицательный размер данных");
    }
    let raw = r.take(n as usize)?;
    let pixels = if lz4 {
        lz4_flex::block::decompress(raw, unpacked).map_err(|e| anyhow!("lz4: {e:?}"))?
    } else {
        raw.to_vec()
    };

    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        bail!("странный размер текстуры {w}x{h}");
    }

    let (w, h, rgba) = if image_format >= 0 {
        let im = image::load_from_memory(&pixels)?.to_rgba8();
        (im.width() as usize, im.height() as usize, im.into_raw())
    } else {
        let need = |k: usize| -> Result<()> {
            if pixels.len() < w * h * k {
                bail!("мало данных: нужно {}, есть {}", w * h * k, pixels.len());
            }
            Ok(())
        };
        let rgba = match format {
            0 => {
                need(4)?;
                pixels[..w * h * 4].to_vec()
            }
            4 => bc(&pixels, w, h, 5)?,
            6 => bc(&pixels, w, h, 3)?,
            7 => bc(&pixels, w, h, 1)?,
            8 => {
                need(2)?;
                pixels.chunks_exact(2).take(w * h).flat_map(|p| [p[0], p[1], 0, 255]).collect()
            }
            9 => {
                need(1)?;
                pixels.iter().take(w * h).flat_map(|&v| [v, v, v, 255]).collect()
            }
            f => bail!("неизвестный формат пикселей {f}"),
        };
        (w, h, rgba)
    };

    // текстура может быть больше самой картинки (степени двойки) — обрезаем
    let (iw, ih) = (iw.max(0) as usize, ih.max(0) as usize);
    if iw > 0 && ih > 0 && iw <= w && ih <= h && (iw, ih) != (w, h) {
        let mut out = Vec::with_capacity(iw * ih * 4);
        for y in 0..ih {
            out.extend_from_slice(&rgba[y * w * 4..(y * w + iw) * 4]);
        }
        return Ok(Image { width: iw as u32, height: ih as u32, rgba: out });
    }
    Ok(Image { width: w as u32, height: h as u32, rgba })
}

fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = (((c >> 11) & 31) as u32, ((c >> 5) & 63) as u32, (c & 31) as u32);
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

fn color_block(b: &[u8], dxt1: bool, px: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32, d: u32| -> [u8; 3] {
        [0usize, 1, 2].map(|i| ((p0[i] as u32 * wa + p1[i] as u32 * wb) / d) as u8)
    };
    let rgba = |c: [u8; 3]| [c[0], c[1], c[2], 255];
    let four = !dxt1 || c0 > c1;
    let pal = if four {
        [rgba(p0), rgba(p1), rgba(mix(2, 1, 3)), rgba(mix(1, 2, 3))]
    } else {
        [rgba(p0), rgba(p1), rgba(mix(1, 1, 2)), [0, 0, 0, 0]]
    };
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (k, p) in px.iter_mut().enumerate() {
        *p = pal[((idx >> (2 * k)) & 3) as usize];
    }
}

fn bc3_alpha(a: &[u8], px: &mut [[u8; 4]; 16]) {
    let (a0, a1) = (a[0] as u32, a[1] as u32);
    let pal: [u32; 8] = if a0 > a1 {
        [a0, a1, (6 * a0 + a1) / 7, (5 * a0 + 2 * a1) / 7, (4 * a0 + 3 * a1) / 7, (3 * a0 + 4 * a1) / 7, (2 * a0 + 5 * a1) / 7, (a0 + 6 * a1) / 7]
    } else {
        [a0, a1, (4 * a0 + a1) / 5, (3 * a0 + 2 * a1) / 5, (2 * a0 + 3 * a1) / 5, (a0 + 4 * a1) / 5, 0, 255]
    };
    let mut bits = 0u64;
    for i in 0..6 {
        bits |= (a[2 + i] as u64) << (8 * i);
    }
    for (k, p) in px.iter_mut().enumerate() {
        p[3] = pal[((bits >> (3 * k)) & 7) as usize] as u8;
    }
}

/// kind: 1 = DXT1 (BC1), 3 = DXT3 (BC2), 5 = DXT5 (BC3)
fn bc(data: &[u8], w: usize, h: usize, kind: u8) -> Result<Vec<u8>> {
    let bs = if kind == 1 { 8 } else { 16 };
    let (bx, by) = ((w + 3) / 4, (h + 3) / 4);
    if data.len() < bx * by * bs {
        bail!("мало данных для DXT");
    }
    let mut out = vec![0u8; w * h * 4];
    for j in 0..by {
        for i in 0..bx {
            let blk = &data[(j * bx + i) * bs..][..bs];
            let mut px = [[0u8; 4]; 16];
            if kind == 1 {
                color_block(blk, true, &mut px);
            } else {
                color_block(&blk[8..], false, &mut px);
                if kind == 3 {
                    for (k, p) in px.iter_mut().enumerate() {
                        p[3] = ((blk[k / 2] >> (4 * (k % 2))) & 15) * 17;
                    }
                } else {
                    bc3_alpha(&blk[..8], &mut px);
                }
            }
            for (k, p) in px.iter().enumerate() {
                let (x, y) = (i * 4 + k % 4, j * 4 + k / 4);
                if x < w && y < h {
                    out[(y * w + x) * 4..][..4].copy_from_slice(p);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn make(ver: u32, format: i32, tex: (i32, i32), img: (i32, i32), lz4: Option<usize>, payload: &[u8]) -> Vec<u8> {
        let mut o = vec![];
        let i = |o: &mut Vec<u8>, v: i32| o.extend(v.to_le_bytes());
        o.extend(b"TEXV0005\0TEXI0001\0");
        for v in [format, 0, tex.0, tex.1, img.0, img.1, 0] {
            i(&mut o, v);
        }
        o.extend(format!("TEXB000{ver}\0").as_bytes());
        i(&mut o, 1); // изображений
        if ver >= 3 {
            i(&mut o, -1); // image_format: сырые пиксели
        }
        i(&mut o, 1); // mip-уровней
        i(&mut o, tex.0);
        i(&mut o, tex.1);
        if ver >= 2 {
            i(&mut o, lz4.is_some() as i32);
            i(&mut o, lz4.unwrap_or(0) as i32);
        }
        i(&mut o, payload.len() as i32);
        o.extend(payload);
        o
    }

    #[test]
    fn rgba8888_plain() {
        let px: Vec<u8> = (0..16).collect();
        let t = decode(&make(3, 0, (2, 2), (2, 2), None, &px)).unwrap();
        assert_eq!((t.width, t.height), (2, 2));
        assert_eq!(t.rgba, px);
    }

    #[test]
    fn rgba8888_v1_and_v2() {
        let px: Vec<u8> = (0..16).collect();
        assert_eq!(decode(&make(1, 0, (2, 2), (2, 2), None, &px)).unwrap().rgba, px);
        assert_eq!(decode(&make(2, 0, (2, 2), (2, 2), None, &px)).unwrap().rgba, px);
    }

    #[test]
    fn lz4_compressed() {
        let px: Vec<u8> = (0..64).map(|v| (v % 7) as u8).collect();
        let packed = lz4_flex::block::compress(&px);
        let t = decode(&make(3, 0, (4, 4), (4, 4), Some(px.len()), &packed)).unwrap();
        assert_eq!(t.rgba, px);
    }

    #[test]
    fn dxt1_red_block() {
        let blk = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
        let t = decode(&make(3, 7, (4, 4), (4, 4), None, &blk)).unwrap();
        assert!(t.rgba.chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn dxt5_alpha() {
        // alpha 0 и 255, все индексы 1 => alpha = a1 = 255; цвет чёрный
        let mut blk = vec![0u8, 255, 0x49, 0x92, 0x24, 0x49, 0x92, 0x24];
        blk.extend([0u8; 8]);
        let t = decode(&make(3, 4, (4, 4), (4, 4), None, &blk)).unwrap();
        assert!(t.rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
    }

    #[test]
    fn crops_to_image_size() {
        let px = vec![7u8; 4 * 4 * 4];
        let t = decode(&make(3, 0, (4, 4), (3, 2), None, &px)).unwrap();
        assert_eq!((t.width, t.height, t.rgba.len()), (3, 2, 24));
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"nope").is_err());
        assert!(decode(&[]).is_err());
    }
}
