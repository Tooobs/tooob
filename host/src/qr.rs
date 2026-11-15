//! An Invite Link drawn as a QR code in the Owner's terminal, for a phone's camera to open (#145).
//! Only ever on stderr, and only when it's a terminal wide enough to show it whole and `NO_COLOR`
//! isn't set: its fixed colours are what make it scan, on a dark terminal or a light one.

/// The QR spec's quiet zone, in modules, on every side.
const QUIET: usize = 4;
/// Dark modules on white, set explicitly: never the terminal's own colours.
const COLOURS: &str = "\x1b[30;107m";
const RESET: &str = "\x1b[0m";

/// `link` encoded as a QR code; None when it can't be.
fn encoded(link: &str) -> Option<qrcode::QrCode> {
    qrcode::QrCode::with_error_correction_level(link, qrcode::EcLevel::L).ok()
}

/// How many columns a drawn code takes, quiet zone and all, `indent` not counted.
fn columns(code: &qrcode::QrCode) -> usize {
    code.width() + 2 * QUIET
}

/// `code` drawn two modules a character cell, each line after `indent`, then a blank line.
fn drawn(code: &qrcode::QrCode, indent: &str) -> String {
    let width = code.width();
    let colors = code.to_colors();
    let size = columns(code);
    // Outside the code is the quiet zone, light.
    let dark = |x: usize, y: usize| {
        let (Some(x), Some(y)) = (x.checked_sub(QUIET), y.checked_sub(QUIET)) else {
            return false;
        };
        x < width && y < width && colors[y * width + x] == qrcode::Color::Dark
    };
    let mut out = String::new();
    for y in (0..size).step_by(2) {
        out.push_str(indent);
        out.push_str(COLOURS);
        for x in 0..size {
            out.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push_str(RESET);
        out.push('\n');
    }
    out.push('\n');
    out
}

/// `link` drawn for the terminal on stderr, `indent` before each line: None when stderr isn't a
/// terminal, `NO_COLOR` is set, or the terminal is narrower than the code, which would wrap.
pub fn for_stderr(link: &str, indent: &str) -> Option<String> {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal()
        || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
    {
        return None;
    }
    let code = encoded(link)?;
    let needs = indent.chars().count() + columns(&code);
    (needs <= usize::from(stderr_columns()?)).then(|| drawn(&code, indent))
}

/// How wide the terminal on stderr is, in columns; None when it doesn't say.
fn stderr_columns() -> Option<u16> {
    // SAFETY: winsize is plain integers, for which all zeros is a valid value.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize, into `size`, which is ours and lives past the call.
    let got = unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) };
    (got == 0 && size.ws_col > 0).then_some(size.ws_col)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "https://k51qzi5uqu5dlvj2baxnqndepeb86cbk3ng7n3i46uzyxzyqj2xjonzllnv0v8.r.tooobs.net/#invite=Zm9vYmFyYmF6cXV4cXV1eGNvcmdlZ3JhdWx0Z2FycGx5";

    /// Read a drawn code back as a phone would: its cells as modules, the grid decoded.
    fn read(drawn: &str, indent: &str) -> String {
        let mut rows: Vec<Vec<bool>> = Vec::new();
        let lines: Vec<&str> = drawn.lines().filter(|l| !l.is_empty()).collect();
        for line in lines {
            let line = line.strip_prefix(indent).expect(line);
            let line = line.strip_prefix(COLOURS).expect(line);
            let line = line.strip_suffix(RESET).expect(line);
            let (top, bottom): (Vec<bool>, Vec<bool>) = line
                .chars()
                .map(|c| match c {
                    '█' => (true, true),
                    '▀' => (true, false),
                    '▄' => (false, true),
                    ' ' => (false, false),
                    other => panic!("not a cell: {other:?}"),
                })
                .unzip();
            rows.push(top);
            rows.push(bottom);
        }
        let size = rows[0].len();
        let mut grid = rqrr::PreparedImage::prepare_from_bitmap(size, size, |x, y| rows[y][x]);
        let grids = grid.detect_grids();
        assert_eq!(grids.len(), 1, "one code");
        grids[0].decode().unwrap().1
    }

    #[test]
    fn a_drawn_link_reads_back_as_the_link() {
        let drawn = drawn(&encoded(LINK).unwrap(), "    ");
        assert_eq!(read(&drawn, "    "), LINK);
        assert!(drawn.ends_with("\n\n"), "a blank line after it: {drawn:?}");
    }
}
