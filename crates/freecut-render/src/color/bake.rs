//! Baking the per-pixel stages into an RGB lattice over the 8-bit input
//! cube (README C52, render-compositor.md §17), and looking pixels up in
//! it with tetrahedral interpolation.

use std::sync::Arc;

use freecut_timeline::model::ColorStack;

use super::eval::Evaluator;

/// How many nodes the lattice has along each axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BakeSize {
    /// 33³: the default.
    #[default]
    N33,
    /// 65³: for steep curves and fine user LUTs (K2, K3).
    N65,
}

impl BakeSize {
    pub const fn nodes(self) -> usize {
        match self {
            BakeSize::N33 => 33,
            BakeSize::N65 => 65,
        }
    }

    /// The size a stack needs. Every K1 curve is gentle enough for 33³.
    pub fn for_stack(_stack: &ColorStack) -> BakeSize {
        BakeSize::N33
    }
}

/// An `n³` grid of RGB results over the input cube, red varying fastest,
/// stored unclamped (later stages may still bring values back into range).
#[derive(Debug, Clone, PartialEq)]
pub struct Lattice {
    n: usize,
    nodes: Vec<[f32; 3]>,
    /// Per 8-bit input value: the lower node index and the fraction toward
    /// the next one, `v·(n − 1)/255`.
    cells: Vec<(u32, f32)>,
}

impl Lattice {
    pub fn size(&self) -> usize {
        self.n
    }

    /// The node at `(r, g, b)`.
    pub fn node(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.nodes[(b * self.n + g) * self.n + r]
    }

    /// The lattice value for an 8-bit input pixel, by tetrahedral
    /// interpolation between the four nodes of the tetrahedron (one of six
    /// per cell) that holds it.
    #[inline]
    pub fn lookup(&self, rgb: [u8; 3]) -> [f32; 3] {
        let (r0, fr) = self.cells[usize::from(rgb[0])];
        let (g0, fg) = self.cells[usize::from(rgb[1])];
        let (b0, fb) = self.cells[usize::from(rgb[2])];
        let n = self.n;
        let base = (b0 as usize * n + g0 as usize) * n + r0 as usize;
        let (dr, dg, db) = (1, n, n * n);
        let at = |o: usize| self.nodes[base + o];
        let c000 = at(0);
        let c111 = at(dr + dg + db);
        // Walk from c000 to c111 along the axes in order of their fraction.
        let (first, second, f1, f2, f3) = if fr > fg {
            if fg > fb {
                (dr, dr + dg, fr, fg, fb)
            } else if fr > fb {
                (dr, dr + db, fr, fb, fg)
            } else {
                (db, db + dr, fb, fr, fg)
            }
        } else if fb > fg {
            (db, db + dg, fb, fg, fr)
        } else if fb > fr {
            (dg, dg + db, fg, fb, fr)
        } else {
            (dg, dg + dr, fg, fr, fb)
        };
        let c1 = at(first);
        let c2 = at(second);
        let mut out = [0.0; 3];
        for i in 0..3 {
            out[i] =
                c000[i] + f1 * (c1[i] - c000[i]) + f2 * (c2[i] - c1[i]) + f3 * (c111[i] - c2[i]);
        }
        out
    }
}

/// The input value of node `i` of `n`.
fn node_value(i: usize, n: usize) -> f32 {
    i as f32 / (n - 1) as f32
}

/// Bakes `stack`'s per-pixel stages (stages 5–7, 12 and the Amount blend
/// in K1) into a lattice of `size`, on up to 8 threads.
pub fn bake(stack: &ColorStack, size: BakeSize) -> Arc<Lattice> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    bake_with(stack, size, threads)
}

/// [`bake`] on `threads` threads (at least one). Every node is evaluated on
/// its own, so the result doesn't depend on the thread count.
pub fn bake_with(stack: &ColorStack, size: BakeSize, threads: usize) -> Arc<Lattice> {
    let n = size.nodes();
    let eval = Evaluator::new(stack);
    let mut nodes = vec![[0.0f32; 3]; n * n * n];
    // One blue plane per unit of work: n planes of n² nodes.
    let plane = n * n;
    // Stages 5–6 act on each channel alone: once per node value.
    let balanced: Vec<[f32; 3]> = (0..n)
        .map(|i| [0, 1, 2].map(|c| eval.balance(c, node_value(i, n))))
        .collect();
    let fill = |b: usize, out: &mut [[f32; 3]]| {
        for g in 0..n {
            for r in 0..n {
                let input = [node_value(r, n), node_value(g, n), node_value(b, n)];
                let pre = [balanced[r][0], balanced[g][1], balanced[b][2]];
                out[g * n + r] = eval.eval_balanced(input, pre);
            }
        }
    };
    let threads = threads.clamp(1, n);
    if threads == 1 {
        for (b, out) in nodes.chunks_mut(plane).enumerate() {
            fill(b, out);
        }
    } else {
        let per = n.div_ceil(threads);
        std::thread::scope(|scope| {
            for (i, band) in nodes.chunks_mut(plane * per).enumerate() {
                let fill = &fill;
                scope.spawn(move || {
                    for (j, out) in band.chunks_mut(plane).enumerate() {
                        fill(i * per + j, out);
                    }
                });
            }
        });
    }
    let cells = (0..=255u32)
        .map(|v| {
            let x = v as f32 * (n - 1) as f32 / 255.0;
            // The top value sits on the last cell's far side.
            let i = (x as u32).min(n as u32 - 2);
            (i, x - i as f32)
        })
        .collect();
    Arc::new(Lattice { n, nodes, cells })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_bake_is_exact_on_every_8_bit_input() {
        let l = bake_with(&ColorStack::default(), BakeSize::N33, 3);
        for v in 0..=255u8 {
            for (r, g, b) in [(v, 0, 255 - v), (v, v, v), (255 - v, v / 2, v)] {
                let out = l.lookup([r, g, b]);
                let q = out.map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                assert_eq!(q, [r, g, b]);
            }
        }
    }

    #[test]
    fn bakes_do_not_depend_on_the_thread_count() {
        let s = ColorStack::default()
            .with(freecut_timeline::model::ColorParamId::Hue, 37)
            .with(freecut_timeline::model::ColorParamId::Contrast, 60);
        assert_eq!(
            bake_with(&s, BakeSize::N33, 1),
            bake_with(&s, BakeSize::N33, 8)
        );
    }
}
