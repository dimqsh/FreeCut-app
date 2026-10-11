//! `.3dl` (CL-016, *`.3dl`*).

use super::error::{LutError, Malformed, Unsupported};
use super::text::{Text, not_a_number, parse_f32, parse_uint, quote, tokens};
use super::{Lut, Lut3d, LutNotice, MAX_3DL_MESH, check_len, trilinear};

/// The largest integer output value (16 bit).
const MAX_CODE: u64 = 65_535;

/// Parses a `.3dl` file.
///
/// The first non-comment line is the input mesh (2..=65 strictly increasing
/// integers from 0, ending at 1023, 4095 or 65535); `n³` lines of three
/// values follow, blue fastest. Integer outputs are scaled by the bit depth
/// their largest value implies (≤ 1023 → 10 bit, ≤ 4095 → 12 bit, else 16
/// bit); a file whose data has a decimal point is a float `.3dl`, taken as
/// 0–1. The table is reordered red-fastest, and a mesh more than one code
/// value away from even spacing is resampled onto an even lattice with a
/// [`LutNotice::MeshResampled`].
///
/// # Errors
/// A [`LutError`] naming the line and the rule the file breaks.
pub fn parse_3dl(bytes: &[u8]) -> Result<Lut, LutError> {
    check_len(bytes.len())?;
    let text = Text::new(bytes);
    let mut lines = text.lines().filter(|(_, content)| !content.is_empty());

    let Some((mesh_line, mesh_content)) = lines.next() else {
        return Err(LutError::malformed(None, Malformed::MissingMesh));
    };
    let mesh = parse_mesh(mesh_content, mesh_line)?;
    let n = mesh.len();

    let expected = (n * n * n) as u64;
    let cap = expected.min((bytes.len() / 6) as u64);
    let mut rows: Vec<[f32; 3]> = Vec::with_capacity(usize::try_from(cap).unwrap_or(0));
    let mut float = false;
    let mut max_code = 0u64;
    for (line, content) in lines {
        let words: Vec<&[u8]> = tokens(content).collect();
        if words.len() != 3 {
            // Validate the values first so a decimal comma or a word is
            // reported as such, not as a count.
            for word in &words {
                parse_value(word, line, &mut float, &mut max_code)?;
            }
            return Err(LutError::malformed(
                Some(line),
                Malformed::ValueCount { found: words.len() },
            ));
        }
        let mut row = [0.0f32; 3];
        for (slot, word) in row.iter_mut().zip(&words) {
            *slot = parse_value(word, line, &mut float, &mut max_code)?;
        }
        rows.push(row);
    }

    if rows.len() as u64 != expected {
        return Err(LutError::unsupported(
            Some(mesh_line),
            Unsupported::MeshMismatch {
                points: n as u32,
                expected,
                found: rows.len() as u64,
            },
        ));
    }

    if !float {
        let scale = match max_code {
            0..=1023 => 1023.0,
            1024..=4095 => 4095.0,
            _ => 65_535.0,
        };
        for row in &mut rows {
            for v in row.iter_mut() {
                *v /= scale;
            }
        }
    }

    // File order is blue fastest, red slowest: line k holds
    // (r, g, b) = (k / n², k / n % n, k % n). Store it red fastest.
    let mut data = vec![[0.0f32; 3]; rows.len()];
    for (k, row) in rows.into_iter().enumerate() {
        let (r, g, b) = (k / (n * n), k / n % n, k % n);
        data[r + n * (g + n * b)] = row;
    }
    // `parse_mesh` bounds `n` to 2..=65 and the row count is `n³`.
    let table = Lut3d::new(n as u32, data)
        .ok_or_else(|| LutError::malformed(Some(mesh_line), Malformed::MeshSize { found: n }))?;

    let mut notices = Vec::new();
    let table = if is_even(&mesh) {
        table
    } else {
        notices.push(LutNotice::MeshResampled { size: n as u32 });
        evened(&table, &mesh)
    };

    Ok(Lut {
        title: None,
        shaper: None,
        cube: Some(table),
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        skipped_keywords: 0,
        notices,
    })
}

/// The input mesh: 2..=65 strictly increasing integers from 0, ending at a
/// 10-, 12- or 16-bit maximum.
fn parse_mesh(content: &[u8], line: u32) -> Result<Vec<u64>, LutError> {
    let words: Vec<&[u8]> = tokens(content).collect();
    // A word such as `3DMESH` or `Mesh` where the mesh belongs: a header
    // form this parser does not read.
    if let Some(first) = words.first()
        && first.iter().any(u8::is_ascii_alphabetic)
        && !std::str::from_utf8(first).is_ok_and(|s| s.parse::<f32>().is_ok())
    {
        return Err(LutError::unsupported(
            Some(line),
            Unsupported::MeshHeader {
                keyword: quote(first),
            },
        ));
    }
    let mesh = words
        .iter()
        .map(|w| parse_uint(w, line))
        .collect::<Result<Vec<u64>, LutError>>()?;
    if !(2..=MAX_3DL_MESH).contains(&mesh.len()) {
        return Err(LutError::malformed(
            Some(line),
            Malformed::MeshSize { found: mesh.len() },
        ));
    }
    if mesh[0] != 0 {
        return Err(LutError::malformed(
            Some(line),
            Malformed::MeshStart { first: mesh[0] },
        ));
    }
    if let Some(i) = (1..mesh.len()).find(|&i| mesh[i] <= mesh[i - 1]) {
        return Err(LutError::malformed(
            Some(line),
            Malformed::MeshNotIncreasing { position: i + 1 },
        ));
    }
    let last = mesh[mesh.len() - 1];
    if !matches!(last, 1023 | 4095 | 65_535) {
        return Err(LutError::unsupported(
            Some(line),
            Unsupported::MeshDepth { last },
        ));
    }
    Ok(mesh)
}

/// One data value: a non-negative integer up to 65535, or (with a decimal
/// point) a finite float, which makes the file a float `.3dl`.
fn parse_value(
    word: &[u8],
    line: u32,
    float: &mut bool,
    max_code: &mut u64,
) -> Result<f32, LutError> {
    if word.contains(&b'.') {
        *float = true;
        return parse_f32(word, line);
    }
    // "-5" and "1e3" are numbers but not whole ones (`parse_uint` says so);
    // anything else is not a number, or uses a decimal comma.
    let numeric = std::str::from_utf8(word).is_ok_and(|s| s.parse::<f32>().is_ok());
    if !numeric {
        return Err(not_a_number(word, line));
    }
    let value = parse_uint(word, line)?;
    if value > MAX_CODE {
        return Err(LutError::malformed(
            Some(line),
            Malformed::ValueTooLarge { value },
        ));
    }
    *max_code = (*max_code).max(value);
    // At most 65535, so exact in `f32`.
    Ok(value as f32)
}

/// Whether every mesh value is within one code value of even spacing:
/// `|mesh[i] − i·last/(n − 1)| ≤ 1`, in integers.
fn is_even(mesh: &[u64]) -> bool {
    let n1 = (mesh.len() - 1) as u64;
    let last = mesh[mesh.len() - 1];
    mesh.iter()
        .enumerate()
        .all(|(i, &m)| (m * n1).abs_diff(i as u64 * last) <= n1)
}

/// Resamples a table whose lattice sits at the mesh positions onto an even
/// lattice of the same size (trilinear between mesh positions).
fn evened(table: &Lut3d, mesh: &[u64]) -> Lut3d {
    let n = mesh.len();
    let last = mesh[n - 1] as f32;
    let positions: Vec<f32> = mesh.iter().map(|&m| m as f32 / last).collect();
    // For each even lattice index: the source cell and the fraction in it.
    let cells: Vec<(usize, f32)> = (0..n)
        .map(|k| {
            let x = k as f32 / (n - 1) as f32;
            let j = (0..n - 1).rfind(|&j| positions[j] <= x).unwrap_or(0);
            let t = (x - positions[j]) / (positions[j + 1] - positions[j]);
            (j, t.clamp(0.0, 1.0))
        })
        .collect();
    let mut data = Vec::with_capacity(n * n * n);
    for &(bj, bt) in &cells {
        for &(gj, gt) in &cells {
            for &(rj, rt) in &cells {
                let at = |r: usize, g: usize, b: usize| table.at(rj + r, gj + g, bj + b);
                let out = trilinear(
                    [
                        [at(0, 0, 0), at(1, 0, 0)],
                        [at(0, 1, 0), at(1, 1, 0)],
                        [at(0, 0, 1), at(1, 0, 1)],
                        [at(0, 1, 1), at(1, 1, 1)],
                    ],
                    [rt, gt, bt],
                );
                data.push(out);
            }
        }
    }
    // Same size and `n³` entries as `table`, so `new` accepts it.
    Lut3d::new(n as u32, data).unwrap_or_else(|| table.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_meshes() {
        let usual: Vec<u64> = (0..17).map(|i| (i * 64).min(1023)).collect();
        assert!(is_even(&usual));
        assert!(is_even(&[0, 512, 1023]));
        assert!(is_even(&[0, 2048, 4095]));
        assert!(!is_even(&[0, 256, 1023]));
        assert!(!is_even(&[0, 509, 1023]));
    }
}
