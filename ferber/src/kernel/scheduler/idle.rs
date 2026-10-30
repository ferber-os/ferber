//! Bounded potential-producer reachability; this does not prove protocol progress.
pub const MAX_TASKS: usize = 64;

pub fn rooted(count: usize, edges: &[u64; MAX_TASKS], roots: u64) -> bool {
    if count == 0 || count > MAX_TASKS {
        return false;
    }
    let all = if count == MAX_TASKS { u64::MAX } else { (1u64 << count) - 1 };
    let mut reached = roots & all;
    loop {
        let previous = reached;
        for (index, producers) in edges.iter().enumerate().take(count) {
            if producers & reached != 0 { reached |= 1u64 << index; }
        }
        if reached == all { return true; }
        if reached == previous { return false; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_chain_and_disconnected_cycles() {
        let mut edges = [0; MAX_TASKS];
        edges[0] = 1 << 1;
        edges[1] = 1 << 2;
        assert!(rooted(3, &edges, 1 << 2));
        assert!(!rooted(3, &edges, 0));
        edges[3] = 1 << 4;
        edges[4] = 1 << 3;
        assert!(!rooted(5, &edges, 1 << 2));
        assert!(rooted(5, &edges, (1 << 2) | (1 << 4)));
    }

    #[test]
    fn full_width_and_invalid_roots() {
        let mut edges = [0; MAX_TASKS];
        for (index, edge) in edges.iter_mut().enumerate().take(MAX_TASKS - 1) {
            *edge = 1u64 << (index + 1);
        }
        assert!(rooted(MAX_TASKS, &edges, 1u64 << 63));
        assert!(!rooted(MAX_TASKS, &edges, 0));
        assert!(!rooted(0, &edges, u64::MAX));
        assert!(!rooted(1, &edges, 1 << 1));
        assert!(!rooted(MAX_TASKS + 1, &edges, u64::MAX));
    }
}
