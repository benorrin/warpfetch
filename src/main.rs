//! warpfetch: a fast system info fetch with a starfield warp intro.
//!
//! Everything is read straight from /proc, /sys and /etc; see `packages` for
//! how slow package databases are kept off the startup path.

mod logos;
mod packages;
mod theme;

use std::env;
use std::ffi::{CString, c_char};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use logos::Logo;
use theme::*;

// The statvfs/ioctl bindings below assume the 64-bit Linux ABI.
#[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
compile_error!("warpfetch only supports 64-bit Linux");

const HELP: &str = "\
warpfetch: a fast system info fetch with a starfield warp intro

Usage: warpfetch [--no-anim] [--logo NAME]

Options:
  --no-anim     Print the info straight away, without the animation
  --logo NAME   Show NAME's logo instead of detecting the distro
  --list-logos  List the available logo names
  -h, --help    Show this help
  -V, --version Show the version

Environment:
  WARPFETCH_NO_ANIM  Set to skip the animation
  WARPFETCH_LOGO     Default for --logo

The animation is also skipped when stdout is not a terminal.
";

const FRAMES: usize = 30;
const REVEAL_FROM: usize = 18;
const FRAME_TIME: Duration = Duration::from_millis(30);
const STARS: usize = 110;

fn main() {
    let mut no_anim = false;
    let mut logo_name = env::var("WARPFETCH_LOGO").ok().filter(|s| !s.is_empty());
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return;
            }
            "-V" | "--version" => {
                println!("warpfetch {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--list-logos" => {
                for logo in logos::LOGOS {
                    println!("{}", logo.name);
                }
                return;
            }
            // Internal: spawned in the background to recount slow package databases.
            "--refresh-packages" => {
                packages::refresh();
                return;
            }
            "--no-anim" => no_anim = true,
            "--logo" => logo_name = args.next(),
            other if other.starts_with("--logo=") => logo_name = Some(other[7..].to_string()),
            other => usage_error(&format!("unknown option '{other}'")),
        }
    }
    let logo = match logo_name {
        Some(name) => logos::by_name(&name)
            .unwrap_or_else(|| usage_error(&format!("no logo named '{name}' (see --list-logos)"))),
        None => logos::detect(&fs::read_to_string("/etc/os-release").unwrap_or_default()),
    };
    let animate =
        !no_anim && env::var_os("WARPFETCH_NO_ANIM").is_none() && io::stdout().is_terminal();

    let gather = thread::spawn(gather_info);
    let (cols, _) = term_size();
    let logo_w = logo.width();

    let mut out = io::stdout().lock();
    if cols < logo_w + 40 {
        // Too narrow for the logo: plain info only.
        let info = gather.join().unwrap_or_default();
        for line in info_lines(&info) {
            let mut c = Canvas::new(cols.saturating_sub(1).max(1), 1);
            c.put_segments(0, 0, &line);
            let _ = out.write_all(c.render().as_bytes());
        }
        return;
    }

    let info_x = 2 + logo_w + 4;
    let width = (cols - 1).min(info_x + 60);

    if !animate {
        let info = gather.join().unwrap_or_default();
        let lines = info_lines(&info);
        let height = logo.art.len().max(lines.len()) + 2;
        let mut c = Canvas::new(width, height);
        draw_scene(&mut c, logo, &lines, info_x, |_, _| true, lines.len());
        let _ = out.write_all(c.render().as_bytes());
        return;
    }

    // Height depends on the info, which we don't have yet; it is always
    // at most 16 lines, so reserve for the larger of that and the logo.
    let height = logo.art.len().max(16) + 2;
    let _ = write!(out, "\x1b[?25l{}\x1b[{}A", "\n".repeat(height), height);

    let mut rng = Rng::seeded();
    let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
    let mut stars: Vec<Star> = (0..STARS).map(|_| Star::new(&mut rng, true)).collect();
    let reveal_at: Vec<f32> = (0..logo_w * logo.art.len()).map(|_| rng.f32()).collect();
    let mut info: Option<Info> = None;
    let mut gather = Some(gather);

    for frame in 0..=FRAMES {
        let start = Instant::now();
        let t = frame as f32 / FRAMES as f32;
        let mut c = Canvas::new(width, height);

        if frame < FRAMES {
            // Once the logo starts to appear, stars that leave are not replaced,
            // so the field empties out and the logo settles onto a clean background.
            let respawn = frame < REVEAL_FROM;
            let speed = 0.012 + 0.07 * t * t;
            for s in stars.iter_mut().filter(|s| s.z > 0.0) {
                let prev = s.project(cx, cy);
                s.z -= speed;
                let cur = s.project(cx, cy);
                if s.z <= 0.03 || !c.contains(cur.0, cur.1) {
                    if respawn {
                        *s = Star::new(&mut rng, false);
                    } else {
                        s.z = -1.0;
                    }
                    continue;
                }
                let in_logo = cur.0 >= 1.0
                    && cur.0 < (logo_w + 3) as f32
                    && cur.1 < (logo.art.len() + 2) as f32;
                if !in_logo || respawn {
                    c.streak(prev, cur, s.z);
                }
            }
        }

        let reveal = if frame < REVEAL_FROM {
            -1.0
        } else {
            (frame - REVEAL_FROM) as f32 / (FRAMES - REVEAL_FROM) as f32
        };
        if reveal >= 0.0 && info.is_none() {
            info = gather.take().map(|g| g.join().unwrap_or_default());
        }
        let lines = info.as_ref().map(info_lines).unwrap_or_default();
        let shown = (reveal.max(0.0) * lines.len() as f32).ceil() as usize;
        draw_scene(
            &mut c,
            logo,
            &lines,
            info_x,
            |x, y| reveal >= 0.0 && reveal_at[y * logo_w + x] < reveal * 1.15,
            shown,
        );

        let _ = write!(out, "{}\x1b[{}A", c.render(), height);
        let _ = out.flush();
        if let Some(rest) = FRAME_TIME.checked_sub(start.elapsed()) {
            thread::sleep(rest);
        }
    }
    let _ = write!(out, "\x1b[{}B\x1b[?25h", height);
    let _ = out.flush();
}

fn usage_error(msg: &str) -> ! {
    eprintln!("warpfetch: {msg}\n\n{HELP}");
    std::process::exit(2);
}

/// Draw the logo (cells for which `revealed` is true) and the first `shown` info lines.
fn draw_scene(
    c: &mut Canvas,
    logo: &Logo,
    lines: &[Vec<Seg>],
    info_x: usize,
    revealed: impl Fn(usize, usize) -> bool,
    shown: usize,
) {
    let n = logo.art.len().saturating_sub(1).max(1) as f32;
    for (y, row) in logo.art.iter().enumerate() {
        let col = lerp(logo.colors.0, logo.colors.1, y as f32 / n);
        for (x, ch) in row.chars().enumerate() {
            if ch != ' ' && revealed(x, y) {
                c.put(2 + x, 1 + y, ch, col, true);
            }
        }
    }
    for (i, line) in lines.iter().take(shown).enumerate() {
        c.clear_row_from(info_x - 1, 1 + i);
        c.put_segments(info_x, 1 + i, line);
    }
}

// ---------------------------------------------------------------- rendering

#[derive(Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    fg: Rgb,
    bold: bool,
}

const BLANK: Cell = Cell {
    ch: ' ',
    fg: FG,
    bold: false,
};

struct Canvas {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Canvas {
            w,
            h,
            cells: vec![BLANK; w * h],
        }
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h
    }

    fn put(&mut self, x: usize, y: usize, ch: char, fg: Rgb, bold: bool) {
        if x < self.w && y < self.h {
            self.cells[y * self.w + x] = Cell { ch, fg, bold };
        }
    }

    fn clear_row_from(&mut self, x: usize, y: usize) {
        for xx in x..self.w {
            self.put(xx, y, ' ', FG, false);
        }
    }

    fn put_segments(&mut self, mut x: usize, y: usize, segs: &[Seg]) {
        for seg in segs {
            for ch in seg.text.chars() {
                self.put(x, y, ch, seg.fg, seg.bold);
                x += 1;
            }
        }
    }

    /// A star moving from `a` to `b`: a dim trail plus a head whose glyph and
    /// brightness depend on how close (small `z`) it is.
    fn streak(&mut self, a: (f32, f32), b: (f32, f32), z: f32) {
        let (dx, dy) = (b.0 - a.0, (b.1 - a.1) * 2.0);
        let trail = if dx.abs() > dy.abs() * 2.0 {
            '-'
        } else if dy.abs() > dx.abs() * 2.0 {
            '|'
        } else if (dx > 0.0) == (dy > 0.0) {
            '\\'
        } else {
            '/'
        };
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize;
        for i in 1..steps {
            let f = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            if self.contains(x, y) {
                self.put(
                    x as usize,
                    y as usize,
                    trail,
                    if z < 0.3 { DEEP_BLUE } else { DARK },
                    false,
                );
            }
        }
        let (head, col) = match z {
            z if z > 0.65 => ('.', DARK),
            z if z > 0.35 => ('+', COMMENT),
            z if z > 0.15 => ('*', BLUE),
            _ => ('*', FG),
        };
        self.put(b.0 as usize, b.1 as usize, head, col, z < 0.15);
    }

    fn render(&self) -> String {
        let mut s = String::with_capacity(self.w * self.h * 4);
        for row in self.cells.chunks(self.w) {
            let mut cur: Option<(Rgb, bool)> = None;
            for cell in row {
                if cell.ch != ' ' && cur != Some((cell.fg, cell.bold)) {
                    let (r, g, b) = cell.fg;
                    let bold = if cell.bold { ";1" } else { "" };
                    s.push_str(&format!("\x1b[0{bold};38;2;{r};{g};{b}m"));
                    cur = Some((cell.fg, cell.bold));
                }
                s.push(cell.ch);
            }
            s.push_str("\x1b[0m\x1b[K\r\n");
        }
        s
    }
}

struct Star {
    x: f32,
    y: f32,
    z: f32,
}

impl Star {
    fn new(rng: &mut Rng, anywhere: bool) -> Self {
        Star {
            x: rng.f32() * 2.0 - 1.0,
            y: rng.f32() * 2.0 - 1.0,
            z: if anywhere { 0.2 + rng.f32() * 0.8 } else { 1.0 },
        }
    }

    fn project(&self, cx: f32, cy: f32) -> (f32, f32) {
        (
            cx + self.x / self.z * cx * 0.5,
            cy + self.y / self.z * cy * 0.5,
        )
    }
}

struct Rng(u64);

impl Rng {
    fn seeded() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1);
        Rng((nanos as u64) | 1)
    }

    fn f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

// ---------------------------------------------------------------- info

#[derive(Default)]
struct Info {
    user: String,
    host: String,
    fields: Vec<(&'static str, String)>,
}

#[derive(Clone)]
struct Seg {
    text: String,
    fg: Rgb,
    bold: bool,
}

fn seg(text: impl Into<String>, fg: Rgb, bold: bool) -> Seg {
    Seg {
        text: text.into(),
        fg,
        bold,
    }
}

fn info_lines(info: &Info) -> Vec<Vec<Seg>> {
    let mut lines = vec![
        vec![
            seg(&info.user, PURPLE, true),
            seg("@", COMMENT, false),
            seg(&info.host, BLUE, true),
        ],
        vec![seg(
            "─".repeat(info.user.len() + 1 + info.host.len()),
            DARK,
            false,
        )],
    ];
    for (label, value) in &info.fields {
        lines.push(vec![
            seg(format!("{label:<10}"), CYAN, true),
            seg(value, FG, false),
        ]);
    }
    lines.push(vec![]);
    lines.push(SWATCHES.iter().map(|&c| seg("███ ", c, false)).collect());
    lines
}

fn gather_info() -> Info {
    let user = env::var("USER").unwrap_or_else(|_| "user".into());
    let host = read("/proc/sys/kernel/hostname").unwrap_or_else(|| "localhost".into());
    let mut fields = Vec::new();
    let mut add = |label, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            fields.push((label, v));
        }
    };
    add("OS", os());
    add("Host", host_model());
    add("Kernel", read("/proc/sys/kernel/osrelease"));
    add("Uptime", uptime());
    add("Packages", packages::summary());
    add("Shell", env::var("SHELL").ok().map(|s| basename(&s)));
    add("Desktop", desktop());
    add("Terminal", terminal());
    add("CPU", cpu());
    add("GPU", gpu());
    add("Memory", memory());
    add("Disk (/)", disk("/"));
    Info { user, host, fields }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

fn os() -> Option<String> {
    let release = fs::read_to_string("/etc/os-release").ok()?;
    let name = release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))?
        .trim_matches('"');
    Some(format!("{name} {}", env::consts::ARCH))
}

fn host_model() -> Option<String> {
    let dmi = |f: &str| read(format!("/sys/devices/virtual/dmi/id/{f}")).filter(|s| !s.is_empty());
    let junk = |s: &str| {
        s.contains("To be filled") || s.contains("System Product") || s.contains("Default string")
    };
    match (dmi("sys_vendor"), dmi("product_name")) {
        (Some(v), Some(p)) if !junk(&p) => Some(format!("{v} {p}")),
        _ => Some(format!("{} {}", dmi("board_vendor")?, dmi("board_name")?)),
    }
}

fn uptime() -> Option<String> {
    let secs = read("/proc/uptime")?
        .split('.')
        .next()?
        .parse::<u64>()
        .ok()?;
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    Some(match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    })
}

fn desktop() -> Option<String> {
    let de = env::var("XDG_CURRENT_DESKTOP").ok()?;
    let de = if de == "KDE" {
        "KDE Plasma".to_string()
    } else {
        de
    };
    match env::var("XDG_SESSION_TYPE") {
        Ok(t) if !t.is_empty() => {
            let mut t = t;
            t[..1].make_ascii_uppercase();
            Some(format!("{de} ({t})"))
        }
        _ => Some(de),
    }
}

fn terminal() -> Option<String> {
    if let Some(v) = env::var("KONSOLE_VERSION")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        return Some(format!(
            "Konsole {}.{:02}.{}",
            v / 10000,
            v / 100 % 100,
            v % 100
        ));
    }
    env::var("TERM_PROGRAM").or_else(|_| env::var("TERM")).ok()
}

fn cpu() -> Option<String> {
    let info = fs::read_to_string("/proc/cpuinfo").ok()?;
    let model = info.lines().find_map(|l| l.strip_prefix("model name"))?;
    let mut model = model.trim_start_matches([' ', '\t', ':']).to_string();
    for junk in ["(R)", "(TM)", "(tm)", " CPU", " Processor"] {
        model = model.replace(junk, "");
    }
    if let Some(i) = model.find(" @") {
        model.truncate(i);
    }
    if let Some(i) = model.find("-Core") {
        // "Eight-Core" / "8-Core": drop the core-count word.
        let start = model[..i].rfind(' ').unwrap_or(i);
        model.replace_range(start..i + 5, "");
    }
    let threads = info.lines().filter(|l| l.starts_with("processor")).count();
    let mhz = read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        .and_then(|k| k.parse::<f32>().ok())
        .map(|k| format!(" @ {:.2} GHz", k / 1_000_000.0))
        .unwrap_or_default();
    Some(format!(
        "{} ({threads}){mhz}",
        model.split_whitespace().collect::<Vec<_>>().join(" ")
    ))
}

fn gpu() -> Option<String> {
    let mut ids: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = entry.path().join("device");
        let id = |f: &str| read(dev.join(f)).map(|s| s.trim_start_matches("0x").to_lowercase());
        if let (Some(v), Some(d)) = (id("vendor"), id("device"))
            && !ids.contains(&(v.clone(), d.clone()))
        {
            ids.push((v, d));
        }
    }
    if ids.is_empty() {
        return None;
    }
    let db = fs::read_to_string("/usr/share/hwdata/pci.ids").unwrap_or_default();
    let names: Vec<String> = ids.iter().map(|(v, d)| pci_name(&db, v, d)).collect();
    Some(names.join(", "))
}

fn pci_name(db: &str, vendor: &str, device: &str) -> String {
    let short = match vendor {
        "10de" => "NVIDIA",
        "1002" => "AMD",
        "8086" => "Intel",
        _ => "",
    };
    let mut in_vendor = false;
    let mut vendor_name = String::new();
    for line in db.lines() {
        if !line.starts_with('\t') && !line.starts_with('#') {
            if in_vendor {
                break;
            }
            if let Some(name) = line.strip_prefix(vendor) {
                in_vendor = true;
                vendor_name = name.trim().to_string();
            }
        } else if in_vendor && line.starts_with('\t') && !line.starts_with("\t\t") {
            let line = &line[1..];
            if let Some(rest) = line.strip_prefix(device) {
                let rest = rest.trim();
                // Prefer the marketing name in brackets: "GA104 [GeForce RTX 3070]".
                let model = match (rest.find('['), rest.rfind(']')) {
                    (Some(a), Some(b)) if b > a => &rest[a + 1..b],
                    _ => rest,
                };
                let v = if short.is_empty() {
                    vendor_name.as_str()
                } else {
                    short
                };
                return format!("{v} {model}");
            }
        }
    }
    format!(
        "{} {vendor}:{device}",
        if short.is_empty() { "PCI" } else { short }
    )
}

fn memory() -> Option<String> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |key: &str| {
        info.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse::<f64>().ok())
    };
    let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    let used = total - avail;
    Some(usage(used * 1024.0, total * 1024.0))
}

fn usage(used: f64, total: f64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!(
        "{:.1} GiB / {:.1} GiB ({:.0}%)",
        used / GIB,
        total / GIB,
        used / total * 100.0
    )
}

#[repr(C)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    spare: [i32; 6],
}

#[repr(C)]
struct WinSize {
    rows: u16,
    cols: u16,
    xpix: u16,
    ypix: u16,
}

unsafe extern "C" {
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

fn disk(path: &str) -> Option<String> {
    let p = CString::new(path).ok()?;
    let mut s: StatVfs = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(p.as_ptr(), &mut s) } != 0 || s.f_blocks == 0 {
        return None;
    }
    let total = (s.f_blocks * s.f_frsize) as f64;
    let used = total - (s.f_bfree * s.f_frsize) as f64;
    Some(usage(used, total))
}

fn term_size() -> (usize, usize) {
    const TIOCGWINSZ: u64 = 0x5413;
    let mut ws = WinSize {
        rows: 0,
        cols: 0,
        xpix: 0,
        ypix: 0,
    };
    if unsafe { ioctl(1, TIOCGWINSZ, &mut ws) } == 0 && ws.cols > 0 {
        (ws.cols as usize, ws.rows as usize)
    } else {
        (80, 24)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCI_IDS: &str = "\
# comment
10de  NVIDIA Corporation
\t13c2  GM204 [GeForce GTX 970]
\t\t1043 8508  Some subsystem
\t1b80  GP104 [GeForce GTX 1080]
1234  Example Vendor
\tabcd  Plain Device
";

    #[test]
    fn pci_name_prefers_bracketed_marketing_name() {
        assert_eq!(pci_name(PCI_IDS, "10de", "13c2"), "NVIDIA GeForce GTX 970");
        assert_eq!(pci_name(PCI_IDS, "10de", "1b80"), "NVIDIA GeForce GTX 1080");
    }

    #[test]
    fn pci_name_falls_back_to_vendor_name_and_ids() {
        assert_eq!(
            pci_name(PCI_IDS, "1234", "abcd"),
            "Example Vendor Plain Device"
        );
        assert_eq!(pci_name(PCI_IDS, "10de", "ffff"), "NVIDIA 10de:ffff");
        assert_eq!(pci_name(PCI_IDS, "9999", "0001"), "PCI 9999:0001");
    }

    #[test]
    fn usage_formats_gib_and_percent() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        assert_eq!(usage(4.0 * gib, 16.0 * gib), "4.0 GiB / 16.0 GiB (25%)");
    }

    #[test]
    fn lerp_hits_endpoints() {
        assert_eq!(lerp(GREEN, TEAL, 0.0), GREEN);
        assert_eq!(lerp(GREEN, TEAL, 1.0), TEAL);
    }

    #[test]
    fn render_emits_one_line_per_row() {
        let mut c = Canvas::new(10, 3);
        c.put(1, 1, '*', BLUE, true);
        let out = c.render();
        assert_eq!(out.matches("\r\n").count(), 3);
        assert!(out.contains("\x1b[0;1;38;2;122;162;247m*"));
    }
}
