# D2R bin to txt

A small Windows program that turns Diablo II: Resurrected excel `.bin` files back into tab-separated `.txt` files.

Drop `.bin` files (or folders of them) on the window or on the `.exe`, or use **Browse...**. Each table is written
next to its `.bin` as `<table>.txt`, plus `<table>.layout.txt` with the column list (name, type, offset in the row,
size, bit).

- One file, about 290 KB, no install. It only uses DLLs that ship with Windows.
- No dependencies: plain Win32 calls, no crates.
- Command line too: `"D2R bin to txt.exe" --cli <files or folders> [--out <folder>]`

## What comes out

D2R `.bin` files describe themselves, so the column names and their order are the real ones from the original
`.txt`. The file also keeps the original text of link and string-key columns (skill names, string keys, sounds),
which is written as is. Empty cells stay empty, like in the game's own `.txt` files.

Columns that are only stored as row numbers into another table (for example `prop1`-`prop12` in uniqueitems are rows
of `properties`) are written as numbers. If the other table's `.bin` (`properties.bin`, `sounds.bin`) is in the same
folder, its codes are written instead.

Files that hold several tables (such as `charstats.bin`, which has two versions of charstats) give
`charstats.txt`, `charstats.2.txt`, and so on.

## The format

```
file header    "D2RLBIN\0" ... at 0x30: table count, then one entry for the file:
               {name offset, name length, table headers offset, table headers size, 0}
table header   one per table, back to back: 8 hash bytes, 4 flag bytes, then dwords:
               rows, record size, descriptors offset/size, column names offset/size,
               rows offset/size, text lookup offset/size, text offset/size
descriptor     16 bytes per column, in .txt column order:
               {u32 name offset, u16 name length, u16 type, u32 extra, u32 offset in the row}
               extra = string length - 1 (type 1) or bit number (type 29)
text lookup    16 bytes per entry: {u32 row, u16 column, u16 type, u32 text offset, u32 length}
```

Column types: 1 string, 2 int32, 3 word, 4/13 byte, 9 four-character code, 15/22 word link, 24 string key,
28 int32 link or formula, 29 bit. Any other type is read by its size and listed in the layout file.

## Building

```
cargo build --release
```

`.cargo/config.toml` links the C runtime statically, so the exe runs without the Visual C++ Redistributable.
