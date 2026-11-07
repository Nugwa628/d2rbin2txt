//! Diablo II: Resurrected excel .bin ("D2RLBIN") -> tab-separated .txt.
//!
//! A D2R .bin describes itself:
//!   file header   "D2RLBIN\0", at 0x30 the table count, then one entry for the file:
//!                 {name offset, name length, table headers offset, table headers size (all tables), 0}
//!   table header  one per table, back to back (e.g. charstats.bin holds two versions of charstats):
//!                 8 hash bytes + 4 flag bytes, then dwords: rows, record size, descriptors offset/size,
//!                 column names offset/size, rows offset/size, text lookup offset/size, text offset/size
//!   descriptor    16 bytes per column, in the original .txt column order:
//!                 {u32 name offset, u16 name length, u16 type, u32 extra, u32 offset in the row}
//!                 extra = string length - 1 (type 1) or bit number (type 29)
//!   text lookup   16 bytes per entry: {u32 row, u16 column, u16 type, u32 text offset, u32 length}: the original
//!                 .txt text of link / string-key cells (skill names, string keys, sounds); length 0 = empty cell
//! Column types: 1 string, 2 int32, 3 word, 4/13 byte, 9 4-char code, 15/22 word link, 24 string key,
//! 28 int32 link / calc, 29 bit; anything else is read with the size up to the next column (at most 4) and
//! flagged in the layout file.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Column {
    pub name: String,
    pub ty: u16,
    pub extra: i32,
    pub offset: usize,
    pub size: usize,
}

pub struct Table {
    pub name: String,
    pub rows: usize,
    pub record: usize,
    pub rows_offset: usize,
    pub columns: Vec<Column>,
    /// the original .txt text of link / string-key cells: (row, column) -> text
    pub texts: HashMap<(usize, usize), String>,
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, String> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| "file is cut short".to_string())
}

fn i32_at(d: &[u8], o: usize) -> Result<i32, String> {
    d.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| "file is cut short".to_string())
}

fn usize_at(d: &[u8], o: usize) -> Result<usize, String> {
    let v = i32_at(d, o)?;
    usize::try_from(v).map_err(|_| format!("bad value {} at 0x{:x}", v, o))
}

pub fn type_name(t: u16) -> &'static str {
    match t {
        1 => "string",
        2 => "int32",
        3 => "word",
        4 | 13 => "byte",
        9 => "code",
        15 | 22 => "word link",
        24 => "string key",
        28 => "int32 link",
        29 => "bit",
        _ => "unknown",
    }
}

fn fixed_size(t: u16, extra: i32) -> usize {
    match t {
        1 => (extra.max(0) + 1) as usize,
        2 | 9 | 28 => 4,
        3 | 22 => 2,
        4 | 13 | 29 => 1,
        _ => 0,
    }
}

pub fn read(d: &[u8]) -> Result<Vec<Table>, String> {
    if d.len() < 0x48 || &d[..7] != b"D2RLBIN" {
        return Err("not a D2R .bin (no D2RLBIN header)".into());
    }
    let count = i32_at(d, 0x30)?;
    if !(1..=64).contains(&count) {
        return Err(format!("unexpected table count {}", count));
    }
    // one entry for the file: {name offset, name length, table headers offset, table headers size (all), 0}
    let (no, nl) = (usize_at(d, 0x34)?, usize_at(d, 0x38)?);
    let base = latin1(d.get(no..no + nl).ok_or("table name outside the file")?);
    let (hoff, hsize) = (usize_at(d, 0x3C)?, usize_at(d, 0x40)?);
    let per = hsize / count as usize;
    if per < 0x30 {
        return Err(format!("unexpected table header size {}", hsize));
    }
    let mut tables = Vec::new();
    for k in 0..count as usize {
        // several tables in one file (e.g. charstats: two versions of it): name, name.2, name.3 ...
        let name = if k == 0 { base.clone() } else { format!("{}.{}", base, k + 1) };
        let h = hoff + per * k + 12; // skip 8 hash bytes + 4 flag bytes
        let rows = usize_at(d, h)?;
        let record = usize_at(d, h + 4)?;
        let (desc_off, desc_size) = (usize_at(d, h + 8)?, usize_at(d, h + 12)?);
        let names_off = usize_at(d, h + 16)?;
        let rows_offset = usize_at(d, h + 24)?;
        if usize_at(d, h + 28)? != rows * record || rows_offset + rows * record > d.len() {
            return Err(format!("{}: rows block does not match rows x record size", name));
        }
        let mut columns = Vec::new();
        let mut o = desc_off;
        while o + 16 <= desc_off + desc_size {
            let (cn, cl) = (names_off + usize_at(d, o)?, u16_at(d, o + 4)? as usize);
            columns.push(Column {
                name: latin1(d.get(cn..cn + cl).ok_or("column name outside the file")?),
                ty: u16_at(d, o + 6)?,
                extra: i32_at(d, o + 8)?,
                offset: usize_at(d, o + 12)?,
                size: 0,
            });
            o += 16;
        }
        let mut starts: Vec<usize> = columns.iter().map(|c| c.offset).chain([record]).collect();
        starts.sort_unstable();
        starts.dedup();
        for c in columns.iter_mut() {
            c.size = fixed_size(c.ty, c.extra);
            if c.size == 0 {
                let next = starts.iter().copied().find(|&x| x > c.offset).unwrap_or(record);
                c.size = (next - c.offset).clamp(1, 4);
            }
            if c.offset + c.size > record {
                return Err(format!("{}: column {} runs past the end of the row", name, c.name));
            }
        }
        // text lookup: 16-byte entries {u32 row, u16 column, u16 type, u32 text offset, u32 length} into a text block
        let (look_off, look_size) = (usize_at(d, h + 32)?, usize_at(d, h + 36)?);
        let (text_off, text_size) = (usize_at(d, h + 40)?, usize_at(d, h + 44)?);
        let mut texts = HashMap::new();
        if look_off + look_size <= d.len() && text_off + text_size <= d.len() {
            let mut o = look_off;
            while o + 16 <= look_off + look_size {
                let (row, col) = (usize_at(d, o)?, u16_at(d, o + 4)? as usize);
                let (to, tl) = (usize_at(d, o + 8)?, usize_at(d, o + 12)?);
                // length 0 = the cell was empty in the .txt (the number in the row is just a default)
                if row < rows && col < columns.len() && to + tl <= text_size {
                    texts.insert((row, col), latin1(&d[text_off + to..text_off + to + tl]));
                }
                o += 16;
            }
        }
        tables.push(Table { name, rows, record, rows_offset, columns, texts });
    }
    Ok(tables)
}

pub fn cell(d: &[u8], row: usize, c: &Column) -> String {
    let o = row + c.offset;
    let b = &d[o..o + c.size];
    match c.ty {
        1 | 9 => {
            let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
            let s = latin1(&b[..n]).replace('\t', " ").replace('\r', "").replace('\n', "\\n");
            if c.ty == 9 { s.trim_end_matches(' ').to_string() } else { s } // codes are space-padded
        }
        29 => if (b[0] >> (c.extra & 7)) & 1 != 0 { "1".into() } else { String::new() },
        15 | 22 => {
            // 22 (e.g. sounds): row 0 is a real row; 15 (item type): 0 = none, like an empty cell
            let v = u16::from_le_bytes([b[0], b[1]]);
            if v == 0xFFFF || (c.ty == 15 && v == 0) { String::new() } else { v.to_string() }
        }
        28 => {
            let v = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if v == -1 { String::new() } else { v.to_string() }
        }
        _ => {
            let v: i64 = match c.size {
                1 => b[0] as i64,
                2 => u16::from_le_bytes([b[0], b[1]]) as i64,
                3 => (b[0] as i64) | (b[1] as i64) << 8 | (b[2] as i64) << 16,
                _ => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64,
            };
            if v == 0 { String::new() } else { v.to_string() } // like the game's .txt files: 0 = empty cell
        }
    }
}

/// Link columns whose target table is known by the column name: the row number becomes the first text cell of
/// that row, when the target's .bin is in the same folder (prop1 = 7 -> the code of properties row 7).
fn link_target(name: &str) -> Option<&'static str> {
    let digits_then = |rest: &str, tail: &[&str]| {
        let n = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        n > 0 && tail.contains(&&rest[n..])
    };
    if name == "prop"
        || name.strip_prefix("prop").map_or(false, |r| digits_then(r, &[""]))
        || name.strip_prefix("tprop").map_or(false, |r| digits_then(r, &[""]))
        || name.strip_prefix("aprop").map_or(false, |r| digits_then(r, &["", "a", "b"]))
    {
        return Some("properties");
    }
    if name == "dropsound" || name == "usesound" {
        return Some("sounds");
    }
    None
}

fn first_column(folder: &Path, table: &str, cache: &mut HashMap<String, Option<Vec<String>>>) -> Option<Vec<String>> {
    if let Some(v) = cache.get(table) {
        return v.clone();
    }
    let got = fs::read(folder.join(format!("{}.bin", table))).ok().and_then(|d| {
        let t = read(&d).ok()?.into_iter().find(|t| t.name.eq_ignore_ascii_case(table))?;
        let c = t.columns.iter().find(|c| c.ty == 1 || c.ty == 9).or(t.columns.first())?;
        Some((0..t.rows).map(|r| cell(&d, t.rows_offset + r * t.record, c)).collect())
    });
    cache.insert(table.to_string(), got.clone());
    got
}

/// Convert one .bin; returns a line per table for the log. Output goes to `out` or next to the .bin.
pub fn convert(path: &Path, out: Option<&Path>) -> Result<Vec<String>, String> {
    let d = fs::read(path).map_err(|e| e.to_string())?;
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let out = out.map(Path::to_path_buf).unwrap_or_else(|| folder.clone());
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut cache = HashMap::new();
    let mut lines = Vec::new();
    for t in read(&d)? {
        let names: Vec<Option<Vec<String>>> = t.columns.iter()
            .map(|c| if c.ty == 22 || c.ty == 28 { link_target(&c.name).and_then(|x| first_column(&folder, x, &mut cache)) } else { None })
            .collect();
        let mut txt = t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join("\t");
        txt.push_str("\r\n");
        for r in 0..t.rows {
            let row = t.rows_offset + r * t.record;
            let cells: Vec<String> = t.columns.iter().zip(&names).enumerate().map(|(ci, (c, n))| {
                if let Some(s) = t.texts.get(&(r, ci)) {
                    return s.replace('\t', " ").replace('\r', "").replace('\n', "\\n"); // the cell's own .txt text
                }
                let v = cell(&d, row, c);
                match (n, v.parse::<usize>()) {
                    (Some(n), Ok(i)) if i < n.len() => n[i].clone(),
                    _ => v,
                }
            }).collect();
            txt.push_str(&cells.join("\t"));
            txt.push_str("\r\n");
        }
        // write as Latin-1, byte for byte like the .bin holds it
        let bytes: Vec<u8> = txt.chars().map(|c| c as u32 as u8).collect();
        fs::write(out.join(format!("{}.txt", t.name)), bytes).map_err(|e| e.to_string())?;

        let mut lay = String::from("column\tname\ttype\ttype name\toffset\tsize\tbit / string length\r\n");
        for (i, c) in t.columns.iter().enumerate() {
            let note = match c.ty { 29 => format!("bit {}", c.extra), 1 => (c.extra + 1).to_string(), _ => String::new() };
            lay.push_str(&format!("{}\t{}\t{}\t{}\t0x{:x}\t{}\t{}\r\n", i + 1, c.name, c.ty, type_name(c.ty), c.offset, c.size, note));
        }
        lay.push_str(&format!("\r\nrows {}, record size {} bytes\r\n", t.rows, t.record));
        let lb: Vec<u8> = lay.chars().map(|c| c as u32 as u8).collect();
        fs::write(out.join(format!("{}.layout.txt", t.name)), lb).map_err(|e| e.to_string())?;

        let with_text: std::collections::HashSet<usize> = t.texts.keys().map(|k| k.1).collect();
        let unknown: Vec<String> = t.columns.iter().enumerate()
            .filter(|(i, c)| type_name(c.ty) == "unknown" && !with_text.contains(i)).map(|(_, c)| c)
            .map(|c| format!("{} (type {})", c.name, c.ty)).collect();
        let named: Vec<&str> = t.columns.iter().zip(&names).filter(|(_, n)| n.is_some()).map(|(c, _)| c.name.as_str()).collect();
        let mut line = format!("{} -> {}.txt  ({} rows, {} columns)", path.file_name().unwrap_or_default().to_string_lossy(),
                               t.name, t.rows, t.columns.len());
        if !named.is_empty() {
            line.push_str(&format!("; names for {}", named.join(", ")));
        }
        if !unknown.is_empty() {
            line.push_str(&format!("; unknown types read by size: {}", unknown.join(", ")));
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Every .bin a dropped path stands for: the file itself, or the .bin files in a folder.
pub fn expand(p: &Path) -> Vec<PathBuf> {
    if p.is_dir() {
        let mut v: Vec<PathBuf> = fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|f| f.extension().map_or(false, |x| x.eq_ignore_ascii_case("bin"))).collect();
        v.sort();
        v
    } else {
        vec![p.to_path_buf()]
    }
}
