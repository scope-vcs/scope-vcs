use std::ops::Range;

const MAX_EDIT_DISTANCE: usize = 256;
const MAX_DIFFED_LINES: usize = 20_000;

pub(super) fn matched_new_index(old: &[&str], new: &[&str], range: Range<usize>) -> Option<usize> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if range.end <= prefix {
        return Some(range.start);
    }
    let old_suffix_start = old.len() - suffix;
    let new_suffix_start = new.len() - suffix;
    if range.start >= old_suffix_start {
        return Some(range.start - old_suffix_start + new_suffix_start);
    }
    let old_middle = &old[prefix..old_suffix_start];
    let new_middle = &new[prefix..new_suffix_start];
    if old_middle.len() + new_middle.len() > MAX_DIFFED_LINES {
        return None;
    }
    let mut matches = vec![None; old.len()];
    for (index, slot) in matches.iter_mut().enumerate().take(prefix) {
        *slot = Some(index);
    }
    for offset in 0..suffix {
        matches[old_suffix_start + offset] = Some(new_suffix_start + offset);
    }
    for (old_index, new_index) in matching_pairs(old_middle, new_middle)? {
        matches[prefix + old_index] = Some(prefix + new_index);
    }
    let start = matches[range.start]?;
    range
        .clone()
        .all(|index| matches[index] == Some(start + index - range.start))
        .then_some(start)
}

fn matching_pairs(old: &[&str], new: &[&str]) -> Option<Vec<(usize, usize)>> {
    let n = old.len() as isize;
    let m = new.len() as isize;
    let max_depth = (old.len() + new.len()).min(MAX_EDIT_DISTANCE) as isize;
    let offset = max_depth + 1;
    let mut furthest = vec![0isize; (2 * offset + 1) as usize];
    let mut trace = Vec::new();
    for depth in 0..=max_depth {
        trace.push(Frontier::capture(&furthest, offset, depth));
        for diagonal in (-depth..=depth).step_by(2) {
            let index = (diagonal + offset) as usize;
            let mut x = if diagonal == -depth
                || (diagonal != depth && furthest[index - 1] < furthest[index + 1])
            {
                furthest[index + 1]
            } else {
                furthest[index - 1] + 1
            };
            let mut y = x - diagonal;
            while x < n && y < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            furthest[index] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m));
            }
        }
    }
    None
}

struct Frontier {
    depth: isize,
    values: Vec<isize>,
}

impl Frontier {
    fn capture(furthest: &[isize], offset: isize, depth: isize) -> Self {
        let low = (offset - depth - 1) as usize;
        let high = (offset + depth + 1) as usize;
        Self {
            depth,
            values: furthest[low..=high].to_vec(),
        }
    }

    fn at(&self, diagonal: isize) -> isize {
        self.values[(diagonal + self.depth + 1) as usize]
    }
}

fn backtrack(trace: &[Frontier], n: isize, m: isize) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for frontier in trace.iter().rev() {
        let depth = frontier.depth;
        let diagonal = x - y;
        let previous_diagonal = if diagonal == -depth
            || (diagonal != depth && frontier.at(diagonal - 1) < frontier.at(diagonal + 1))
        {
            diagonal + 1
        } else {
            diagonal - 1
        };
        let previous_x = frontier.at(previous_diagonal);
        let previous_y = previous_x - previous_diagonal;
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
            pairs.push((x as usize, y as usize));
        }
        if depth > 0 {
            x = previous_x;
            y = previous_y;
        }
    }
    pairs.reverse();
    pairs
}
