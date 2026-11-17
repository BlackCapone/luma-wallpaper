//! Контейнер scene.pkg из Wallpaper Engine.
//! Формат (little-endian): u32 длина + строка-магия ("PKGV…"), u32 число файлов,
//! затем для каждого файла: u32 длина + имя, u32 смещение, u32 размер.
//! Данные идут сразу за заголовком, смещения считаются от его конца.
use anyhow::{anyhow, bail, Result};

pub struct Entry {
    pub name: String,
    pub offset: usize,
    pub len: usize,
}

pub struct Pkg {
    data: Vec<u8>,
    base: usize,
    pub entries: Vec<Entry>,
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
    fn u32(&mut self) -> Result<usize> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.u32()?;
        if n > 1024 {
            bail!("слишком длинная строка в заголовке");
        }
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

impl Pkg {
    pub fn parse(data: Vec<u8>) -> Result<Pkg> {
        let (entries, base) = {
            let mut r = R { b: &data, p: 0 };
            let magic = r.string()?;
            if !magic.starts_with("PKGV") {
                bail!("это не .pkg (магия: {magic:?})");
            }
            let count = r.u32()?;
            if count > 200_000 {
                bail!("слишком много файлов: {count}");
            }
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                let name = r.string()?;
                let offset = r.u32()?;
                let len = r.u32()?;
                entries.push(Entry { name, offset, len });
            }
            (entries, r.p)
        };
        for e in &entries {
            let end = base.checked_add(e.offset).and_then(|s| s.checked_add(e.len));
            if end.map_or(true, |end| end > data.len()) {
                bail!("файл {} выходит за пределы архива", e.name);
            }
        }
        Ok(Pkg { data, base, entries })
    }

    pub fn bytes(&self, e: &Entry) -> &[u8] {
        &self.data[self.base + e.offset..self.base + e.offset + e.len]
    }

    #[allow(dead_code)]
    pub fn find(&self, name: &str) -> Option<&[u8]> {
        self.entries.iter().find(|e| e.name == name).map(|e| self.bytes(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = vec![];
        let magic = b"PKGV0001";
        out.extend((magic.len() as u32).to_le_bytes());
        out.extend(magic);
        out.extend((files.len() as u32).to_le_bytes());
        let mut off = 0u32;
        for (n, d) in files {
            out.extend((n.len() as u32).to_le_bytes());
            out.extend(n.as_bytes());
            out.extend(off.to_le_bytes());
            out.extend((d.len() as u32).to_le_bytes());
            off += d.len() as u32;
        }
        for (_, d) in files {
            out.extend(*d);
        }
        out
    }

    #[test]
    fn parses_entries() {
        let p = Pkg::parse(build(&[("scene.json", b"{}"), ("materials/a.tex", b"abc")])).unwrap();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.find("scene.json").unwrap(), b"{}");
        assert_eq!(p.find("materials/a.tex").unwrap(), b"abc");
        assert!(p.find("nope").is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Pkg::parse(vec![1, 2, 3]).is_err());
        let mut b = build(&[("a", b"xyz")]);
        b.truncate(b.len() - 1);
        assert!(Pkg::parse(b).is_err());
    }
}
