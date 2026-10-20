//! Facts between terms: `a - b <= c` for term nodes `a` and `b`, kept closed (every fact the
//! others imply through other terms is written down). A term's bounds are its range, kept in the
//! state beside the facts (`state.rs`), so facts stay few and each change costs little.
//! Constants are `i128`s, and a fact whose constant is larger than 2^66 in size is dropped, so
//! adding up to three of them while closing can't overflow.

/// The node that is the number zero: a linear form over it is a constant. It has no facts.
pub const ZERO: usize = 0;

/// No fact.
const NONE: i128 = i128::MAX;

/// The largest constant a fact keeps.
const LIMIT: i128 = 1 << 66;

/// The facts at one point: an `n` by `n` matrix, `m[a * n + b]` the least `c` known with
/// `a - b <= c` (`NONE` when nothing is known), or no matrix at all while there are no facts
/// (most states have none, and copying them is then cheap). It lives on the heap.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    n: usize,
    m: Vec<i128>,
    /// How many facts there are.
    count: usize,
    /// For each node `a`, the nodes `b` with a fact `a - b <= ..` (`rows`) and with `b - a <= ..`
    /// (`cols`), as bits: so walking the facts costs what there are.
    rows: Vec<u128>,
    cols: Vec<u128>,
}

/// The positions of the bits set in `x`.
fn bits(mut x: u128) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        (x != 0).then(|| {
            let i = x.trailing_zeros() as usize;
            x &= x - 1;
            i
        })
    })
}

impl Facts {
    /// No facts over `n` nodes (zero included).
    pub fn new(n: usize) -> Facts {
        assert!(n <= 128, "facts are kept over at most 128 nodes");
        Facts { n, m: Vec::new(), count: 0, rows: Vec::new(), cols: Vec::new() }
    }

    pub fn nodes(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The least `c` known with `a - b <= c` (`a` and `b` different).
    pub fn get(&self, a: usize, b: usize) -> Option<i128> {
        if self.count == 0 {
            return None;
        }
        let c = self.m[a * self.n + b];
        (c != NONE).then_some(c)
    }

    /// Whether the facts say `a - b <= c`.
    pub fn implies(&self, a: usize, b: usize, c: i128) -> bool {
        if a == b {
            return c >= 0;
        }
        self.get(a, b).is_some_and(|k| k <= c)
    }

    /// The nodes `i` (other than `a`) with a fact `i - a <= ..`.
    pub fn towards(&self, a: usize) -> impl Iterator<Item = (usize, i128)> + '_ {
        let mask = self.cols.get(a).copied().unwrap_or(0);
        bits(mask).map(move |i| (i, self.m[i * self.n + a]))
    }

    /// The nodes `j` (other than `b`) with a fact `b - j <= ..`.
    pub fn away(&self, b: usize) -> impl Iterator<Item = (usize, i128)> + '_ {
        let mask = self.rows.get(b).copied().unwrap_or(0);
        bits(mask).map(move |j| (j, self.m[b * self.n + j]))
    }

    /// Every fact `(a, b, c)`.
    pub fn all(&self) -> impl Iterator<Item = (usize, usize, i128)> + '_ {
        self.rows.iter().enumerate().flat_map(move |(a, &mask)| bits(mask).map(move |b| (a, b, self.m[a * self.n + b])))
    }

    /// Sets entry `at` (never on the diagonal) to `v` (`NONE` for no fact), keeping the count.
    fn store(&mut self, at: usize, v: i128) {
        if self.m.is_empty() {
            if v == NONE {
                return;
            }
            self.m = vec![NONE; self.n * self.n];
            self.rows = vec![0; self.n];
            self.cols = vec![0; self.n];
        }
        let (a, b) = (at / self.n, at % self.n);
        match (self.m[at] == NONE, v == NONE) {
            (true, false) => {
                self.count += 1;
                self.rows[a] |= 1 << b;
                self.cols[b] |= 1 << a;
            }
            (false, true) => {
                self.count -= 1;
                self.rows[a] &= !(1 << b);
                self.cols[b] &= !(1 << a);
            }
            _ => {}
        }
        self.m[at] = v;
        if self.count == 0 {
            *self = Facts::new(self.n);
        }
    }

    /// Adds `a - b <= c` and everything it implies with the others: how much work that took (the
    /// entries it looked at), or `None` when the facts contradict each other: no point has them
    /// all (the caller marks the state unreachable).
    pub fn add(&mut self, a: usize, b: usize, c: i128) -> Option<u64> {
        if a == b {
            return (c >= 0).then_some(0);
        }
        if c.abs() > LIMIT || self.implies(a, b, c) {
            return Some(0);
        }
        if self.get(b, a).is_some_and(|back| back + c < 0) {
            return None;
        }
        // Every `i - a <= x` and `b - j <= y` now give `i - j <= x + c + y`. The facts were
        // closed and `b - a + c >= 0`, so the entries this reads don't change on the way.
        let rows: Vec<(usize, i128)> = std::iter::once((a, 0)).chain(self.towards(a)).collect();
        let cols: Vec<(usize, i128)> = std::iter::once((b, 0)).chain(self.away(b)).collect();
        for &(i, x) in &rows {
            for &(j, y) in &cols {
                if i != j {
                    self.lower(i, j, x + c + y);
                }
            }
        }
        Some((2 * self.n + rows.len() * cols.len()) as u64)
    }

    /// Records `i - j <= v` when it says more than what is known (and isn't too large).
    fn lower(&mut self, i: usize, j: usize, v: i128) {
        if v.abs() <= LIMIT && self.get(i, j).is_none_or(|old| v < old) {
            self.store(i * self.n + j, v);
        }
    }

    /// Closes the facts again (after a join, which may leave them open); how much work that took.
    pub fn close(&mut self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let mut work = (self.n * self.n) as u64;
        for k in 0..self.n {
            let rows: Vec<(usize, i128)> = self.towards(k).collect();
            if rows.is_empty() {
                continue;
            }
            let cols: Vec<(usize, i128)> = self.away(k).collect();
            work += (2 * self.n + rows.len() * cols.len()) as u64;
            for &(i, x) in &rows {
                for &(j, y) in &cols {
                    if i != j {
                        self.lower(i, j, x + y);
                    }
                }
            }
        }
        work
    }

    /// Nothing is known about `t` any more.
    pub fn forget(&mut self, t: usize) {
        if self.count == 0 {
            return;
        }
        let (out, into, n) = (self.rows[t], self.cols[t], self.n);
        for j in bits(out) {
            self.store(t * n + j, NONE);
        }
        for i in bits(into) {
            if self.count > 0 {
                self.store(i * n + t, NONE);
            }
        }
    }

    /// `t` became `t + by`: every fact about it moves with it.
    pub fn shift(&mut self, t: usize, by: i128) {
        if self.count == 0 {
            return;
        }
        let moved = |c: i128, by: i128| if (c + by).abs() > LIMIT { NONE } else { c + by };
        let (out, into, n) = (self.rows[t], self.cols[t], self.n);
        for j in bits(out) {
            let v = moved(self.m[t * n + j], by);
            self.store(t * n + j, v);
        }
        for i in bits(into) {
            if self.count > 0 {
                let v = moved(self.m[i * n + t], -by);
                self.store(i * n + t, v);
            }
        }
    }

    /// Sets `a - b <= c` as it is (`None`: no fact), for a join or a widening to build its result.
    pub fn put(&mut self, a: usize, b: usize, c: Option<i128>) {
        if a != b {
            let c = c.filter(|c| c.abs() <= LIMIT).unwrap_or(NONE);
            self.store(a * self.n + b, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_finds_what_two_facts_imply() {
        let mut f = Facts::new(4);
        assert!(f.add(1, 2, -1).is_some()); // 1 < 2
        assert!(f.add(2, 3, 0).is_some()); // 2 <= 3
        assert!(f.implies(1, 3, -1));
        assert!(!f.implies(1, 3, -2));
        assert!(f.add(3, 1, 0).is_none(), "3 <= 1 contradicts 1 < 3");
    }

    #[test]
    fn forgetting_and_shifting_move_only_their_node() {
        let mut f = Facts::new(3);
        f.add(1, 2, -1);
        f.shift(1, 1); // 1 = 1 + 1
        assert!(f.implies(1, 2, 0) && !f.implies(1, 2, -1));
        f.forget(2);
        assert_eq!(f.get(1, 2), None);
        assert!(f.is_empty() && f == Facts::new(3), "no facts compare equal however they got there");
    }

    #[test]
    fn huge_constants_are_dropped() {
        let mut f = Facts::new(4);
        assert!(f.add(1, 2, LIMIT + 1).is_some());
        assert_eq!(f.get(1, 2), None);
        f.add(1, 2, LIMIT);
        f.add(2, 3, LIMIT);
        assert_eq!(f.get(1, 3), None, "the sum passes the limit");
    }

    #[test]
    fn closing_after_puts() {
        let mut f = Facts::new(4);
        f.put(1, 2, Some(1));
        f.put(2, 3, Some(2));
        f.close();
        assert_eq!(f.get(1, 3), Some(3));
        assert_eq!(f.all().count(), 3);
    }
}
