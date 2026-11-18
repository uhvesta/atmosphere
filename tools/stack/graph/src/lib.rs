//! The stack model, with no I/O. A stack is pull requests chained by base
//! branch: the base chain is the truth about the stack. A pull request whose
//! base is the trunk is a root; one whose base is another pull request's head
//! sits under it. Head branch names stay stable across amends, so the same
//! pull requests give the same stack after every upload.

/// One pull request, as far as the stack is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    /// The head branch name.
    pub head: String,
    /// The base branch name: the trunk or another pull request's head.
    pub base: String,
    pub title: String,
    /// The head branch lives in a fork, so no other pull request's base can
    /// be it: such a pull request can sit under another, never over one.
    pub from_fork: bool,
}

/// A pull request placed in the stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub pr: Pr,
    /// 0 for a root.
    pub depth: usize,
    /// The pull request whose head is this one's base; None for a root.
    pub parent: Option<u64>,
}

/// Pull requests ordered as a forest: each root, then its subtree, depth first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    nodes: Vec<Node>,
}

impl Stack {
    /// Builds the stack from pull requests in any order.
    ///
    /// A pull request is a root when its base is not the head of another
    /// pull request in `prs` (usually: its base is the trunk). Roots come in
    /// order of number; children under a parent come in order of number. When
    /// two pull requests share a head, the lower number is used as the parent.
    /// A pull request from a fork is never a parent.
    /// A base chain that loops (which GitHub doesn't allow, but data may be
    /// stale) is broken at the lowest number in the loop, which becomes a
    /// root. Every pull request appears exactly once.
    pub fn new(prs: &[Pr]) -> Stack {
        let mut sorted: Vec<Pr> = prs.to_vec();
        sorted.sort_by_key(|pr| pr.number);

        let mut parent = parents(&sorted);
        break_loops(&mut parent);

        let children = children_lists(&parent);
        let mut visited = vec![false; sorted.len()];
        let mut nodes = Vec::with_capacity(sorted.len());
        for (root, _) in parent.iter().enumerate().filter(|(_, up)| up.is_none()) {
            subtree(&sorted, &parent, &children, root, &mut visited, &mut nodes);
        }
        Stack { nodes }
    }

    /// Every node, in tree order (pre-order, depth first).
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// The node for a pull request number.
    pub fn node(&self, number: u64) -> Option<&Node> {
        self.nodes.iter().find(|node| node.pr.number == number)
    }

    /// The tree that holds `number`: its root and every node under that root,
    /// in tree order. Empty when the number isn't in the stack.
    pub fn tree_of(&self, number: u64) -> Vec<&Node> {
        let Some(mut at) = self.nodes.iter().position(|node| node.pr.number == number) else {
            return Vec::new();
        };
        for _ in 0..self.nodes.len() {
            let Some(up) = self.nodes[at].parent else {
                break;
            };
            match self.nodes.iter().position(|node| node.pr.number == up) {
                Some(index) => at = index,
                None => break,
            }
        }
        let end = self.nodes[at + 1..]
            .iter()
            .position(|node| node.depth == 0)
            .map_or(self.nodes.len(), |offset| at + 1 + offset);
        self.nodes[at..end].iter().collect()
    }

    /// The chains to link as native GitHub stacks, each bottom to top as
    /// pull request numbers. GitHub's native stacks are linear, so each root
    /// gives one chain: the root, its lowest-numbered child, that child's
    /// lowest-numbered child, and so on. Chains of one pull request are left
    /// out; other branches of a tree still chain by base branch only.
    pub fn native_chains(&self) -> Vec<Vec<u64>> {
        let mut chains = Vec::new();
        for root in self.nodes.iter().filter(|node| node.depth == 0) {
            let mut last = root.pr.number;
            let mut chain = vec![last];
            while let Some(child) = self.lowest_child(last) {
                chain.push(child);
                last = child;
            }
            if chain.len() >= 2 {
                chains.push(chain);
            }
        }
        chains
    }

    /// The lowest-numbered child of the pull request with `number`, if any.
    fn lowest_child(&self, number: u64) -> Option<u64> {
        self.nodes
            .iter()
            .filter(|node| node.parent == Some(number))
            .map(|node| node.pr.number)
            .min()
    }
}

/// Where a [`RepoStack`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// GitHub's own stacked pull requests.
    Native,
    /// Worked out from the pull requests' base branches.
    BaseChain,
}

/// The stack that holds one pull request in one repository, bottom first:
/// the model a review of a stacked pull request reads. Unlike [`Stack`], it
/// is one tree, and a base loop means no stack rather than a broken one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStack {
    repo: String,
    trunk: String,
    source: Source,
    github_number: Option<u64>,
    entries: Vec<Node>,
}

impl RepoStack {
    /// The tree that holds `number`, worked out from base branches: its root
    /// first, then each subtree depth first, lower-numbered child first.
    ///
    /// None when `number` isn't in `prs`, when it is alone (no parent and no
    /// children), or when its chain of parents loops. A pull request from a
    /// fork is never a parent.
    pub fn from_base_chain(repo: &str, number: u64, prs: &[Pr]) -> Option<RepoStack> {
        let mut sorted: Vec<Pr> = prs.to_vec();
        sorted.sort_by_key(|pr| pr.number);
        let parent = parents(&sorted);
        let mut root = sorted.iter().position(|pr| pr.number == number)?;
        let mut seen = vec![false; sorted.len()];
        seen[root] = true;
        while let Some(up) = parent[root] {
            if seen[up] {
                return None;
            }
            seen[up] = true;
            root = up;
        }

        let children = children_lists(&parent);
        let mut visited = vec![false; sorted.len()];
        let mut entries = Vec::new();
        subtree(
            &sorted,
            &parent,
            &children,
            root,
            &mut visited,
            &mut entries,
        );
        if entries.len() < 2 {
            return None;
        }
        Some(RepoStack {
            repo: repo.to_owned(),
            trunk: sorted[root].base.clone(),
            source: Source::BaseChain,
            github_number: None,
            entries,
        })
    }

    /// GitHub's native stack `github_number`, whose pull requests are
    /// `members` in any order: ordered trunk up by their base branches, since
    /// the base chain is the truth.
    ///
    /// None, so the caller falls back to [`RepoStack::from_base_chain`], when
    /// there are fewer than two members, a member isn't in `prs`, or the
    /// members don't form exactly one linear chain (each member's base the
    /// previous member's head, none of those from a fork).
    pub fn native(
        repo: &str,
        github_number: u64,
        members: &[u64],
        prs: &[Pr],
    ) -> Option<RepoStack> {
        let mut numbers = members.to_vec();
        numbers.sort_unstable();
        numbers.dedup();
        if numbers.len() < 2 {
            return None;
        }
        let chain = numbers
            .iter()
            .map(|number| prs.iter().find(|pr| pr.number == *number))
            .collect::<Option<Vec<&Pr>>>()?;
        let below = |pr: &Pr| {
            chain
                .iter()
                .filter(|other| {
                    other.number != pr.number && other.head == pr.base && !other.from_fork
                })
                .count()
        };
        let mut bottoms = chain.iter().copied().filter(|&pr| below(pr) == 0);
        let mut at = bottoms.next()?;
        if bottoms.next().is_some() {
            return None;
        }
        let mut entries = vec![Node {
            pr: at.clone(),
            depth: 0,
            parent: None,
        }];
        while entries.len() < chain.len() {
            let mut above = chain
                .iter()
                .filter(|pr| pr.number != at.number && pr.base == at.head && !at.from_fork);
            let next = *above.next()?;
            if above.next().is_some() || below(next) != 1 {
                return None;
            }
            entries.push(Node {
                pr: next.clone(),
                depth: entries.len(),
                parent: Some(at.number),
            });
            at = next;
        }
        let trunk = entries.first().map(|node| node.pr.base.clone())?;
        Some(RepoStack {
            repo: repo.to_owned(),
            trunk,
            source: Source::Native,
            github_number: Some(github_number),
            entries,
        })
    }

    /// The repository, as given.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// The branch the bottom pull request is based on.
    pub fn trunk(&self) -> &str {
        &self.trunk
    }

    pub fn source(&self) -> Source {
        self.source
    }

    /// GitHub's number for a native stack; None for a base chain.
    pub fn github_number(&self) -> Option<u64> {
        self.github_number
    }

    /// Every pull request in the stack, bottom first.
    pub fn entries(&self) -> &[Node] {
        &self.entries
    }

    /// The pull request under `number`; None for the bottom one or a number
    /// not in the stack.
    pub fn parent_of(&self, number: u64) -> Option<u64> {
        self.entries
            .iter()
            .find(|node| node.pr.number == number)
            .and_then(|node| node.parent)
    }

    /// The pull requests based on `number`, lowest number first; empty for a
    /// number not in the stack.
    pub fn children_of(&self, number: u64) -> Vec<u64> {
        let mut children: Vec<u64> = self
            .entries
            .iter()
            .filter(|node| node.parent == Some(number))
            .map(|node| node.pr.number)
            .collect();
        children.sort_unstable();
        children
    }
}

/// Each pull request's parent, as an index into `sorted` (number order): the
/// lowest-numbered other pull request, not from a fork, whose head is its
/// base. Loops are left in.
fn parents(sorted: &[Pr]) -> Vec<Option<usize>> {
    sorted
        .iter()
        .enumerate()
        .map(|(x, pr)| {
            sorted
                .iter()
                .enumerate()
                .find(|(y, other)| *y != x && !other.from_fork && other.head == pr.base)
                .map(|(y, _)| y)
        })
        .collect()
}

/// Each index's children, in number order.
fn children_lists(parent: &[Option<usize>]) -> Vec<Vec<usize>> {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); parent.len()];
    for (index, up) in parent.iter().copied().enumerate() {
        if let Some(up) = up {
            children[up].push(index);
        }
    }
    children
}

/// Pushes `root` and everything under it onto `nodes`, depth first, lower
/// child first, each index once.
fn subtree(
    sorted: &[Pr],
    parent: &[Option<usize>],
    children: &[Vec<usize>],
    root: usize,
    visited: &mut [bool],
    nodes: &mut Vec<Node>,
) {
    let mut pending: Vec<(usize, usize)> = vec![(root, 0)];
    while let Some((index, depth)) = pending.pop() {
        if visited[index] {
            continue;
        }
        visited[index] = true;
        nodes.push(Node {
            pr: sorted[index].clone(),
            depth,
            parent: parent[index].map(|up| sorted[up].number),
        });
        for &child in children[index].iter().rev() {
            pending.push((child, depth + 1));
        }
    }
}

/// Breaks every parent loop: while some pull request's parent chain comes
/// back to itself, the lowest-numbered pull request on that loop becomes a
/// root. Indices follow number order, so the lowest index is the lowest number.
fn break_loops(parent: &mut [Option<usize>]) {
    loop {
        let mut lowest = None;
        for start in 0..parent.len() {
            if let Some(cycle) = cycle_through(parent, start) {
                lowest = cycle.into_iter().min();
                break;
            }
        }
        match lowest {
            Some(index) => parent[index] = None,
            None => break,
        }
    }
}

/// The pull requests on the parent loop that `start` is part of, if `start`
/// is on one. A chain that only hangs off a loop returns None.
fn cycle_through(parent: &[Option<usize>], start: usize) -> Option<Vec<usize>> {
    let mut members = Vec::new();
    let mut current = parent[start];
    for _ in 0..parent.len() {
        let index = current?;
        members.push(index);
        if index == start {
            return Some(members);
        }
        current = parent[index];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, head: &str, base: &str) -> Pr {
        Pr {
            number,
            head: head.to_owned(),
            base: base.to_owned(),
            title: format!("pr {number}"),
            from_fork: false,
        }
    }

    fn fork(number: u64, head: &str, base: &str) -> Pr {
        Pr {
            from_fork: true,
            ..pr(number, head, base)
        }
    }

    fn entry_numbers(stack: &RepoStack) -> Vec<u64> {
        stack.entries().iter().map(|node| node.pr.number).collect()
    }

    /// Number, depth and parent of every node, in tree order.
    fn summary(stack: &Stack) -> Vec<(u64, usize, Option<u64>)> {
        stack
            .nodes()
            .iter()
            .map(|node| (node.pr.number, node.depth, node.parent))
            .collect()
    }

    fn numbers(nodes: &[&Node]) -> Vec<u64> {
        nodes.iter().map(|node| node.pr.number).collect()
    }

    #[test]
    fn linear_chain_in_shuffled_order_comes_back_in_base_order() {
        let stack = Stack::new(&[
            pr(3, "c", "b"),
            pr(1, "a", "main"),
            pr(2, "b", "a"),
        ]);

        assert_eq!(
            summary(&stack),
            vec![(1, 0, None), (2, 1, Some(1)), (3, 2, Some(2))]
        );
    }

    #[test]
    fn separate_roots_come_in_number_order() {
        let stack = Stack::new(&[pr(5, "e", "main"), pr(2, "b", "main")]);

        assert_eq!(summary(&stack), vec![(2, 0, None), (5, 0, None)]);
    }

    #[test]
    fn tree_is_pre_order_with_lower_child_first() {
        // 1 is the root; 2 and 3 sit under it; 4 sits under 2.
        let stack = Stack::new(&[
            pr(4, "d", "b"),
            pr(3, "c", "a"),
            pr(2, "b", "a"),
            pr(1, "a", "main"),
        ]);

        assert_eq!(
            summary(&stack),
            vec![(1, 0, None), (2, 1, Some(1)), (4, 2, Some(2)), (3, 1, Some(1))]
        );
    }

    #[test]
    fn native_chains_follow_the_lowest_child_only() {
        let stack = Stack::new(&[
            pr(4, "d", "b"),
            pr(3, "c", "a"),
            pr(2, "b", "a"),
            pr(1, "a", "main"),
        ]);

        assert_eq!(stack.native_chains(), vec![vec![1, 2, 4]]);
    }

    #[test]
    fn tree_of_a_grandchild_is_its_whole_tree_only() {
        let stack = Stack::new(&[
            pr(4, "d", "b"),
            pr(3, "c", "a"),
            pr(2, "b", "a"),
            pr(1, "a", "main"),
            pr(11, "q", "p"),
            pr(10, "p", "main"),
        ]);

        assert_eq!(numbers(&stack.tree_of(4)), vec![1, 2, 4, 3]);
        assert_eq!(numbers(&stack.tree_of(11)), vec![10, 11]);
    }

    #[test]
    fn tree_of_an_unknown_number_is_empty() {
        let stack = Stack::new(&[pr(1, "a", "main")]);

        assert!(stack.tree_of(99).is_empty());
    }

    #[test]
    fn single_pull_request_has_no_native_chain() {
        let stack = Stack::new(&[pr(7, "a", "main")]);

        assert_eq!(summary(&stack), vec![(7, 0, None)]);
        assert!(stack.native_chains().is_empty());
    }

    #[test]
    fn base_loop_is_broken_at_the_lowest_number() {
        let stack = Stack::new(&[pr(2, "b", "a"), pr(1, "a", "b")]);

        assert_eq!(summary(&stack), vec![(1, 0, None), (2, 1, Some(1))]);
    }

    #[test]
    fn a_fork_head_is_never_a_parent_in_the_cli_stack() {
        let stack = Stack::new(&[fork(1, "a", "main"), pr(2, "b", "a")]);

        assert_eq!(summary(&stack), vec![(1, 0, None), (2, 0, None)]);
    }

    #[test]
    fn a_native_stack_is_ordered_trunk_up() {
        let prs = [
            pr(3, "c", "b"),
            pr(1, "a", "main"),
            pr(2, "b", "a"),
            pr(9, "z", "main"),
        ];

        let stack = RepoStack::native("o/r", 41, &[3, 1, 2], &prs).expect("one chain");

        assert_eq!(entry_numbers(&stack), vec![1, 2, 3]);
        assert_eq!(stack.trunk(), "main");
        assert_eq!(stack.repo(), "o/r");
        assert_eq!(stack.source(), Source::Native);
        assert_eq!(stack.github_number(), Some(41));
        assert_eq!(stack.parent_of(3), Some(2));
        assert_eq!(stack.children_of(1), vec![2]);
    }

    #[test]
    fn a_native_stack_that_is_not_one_chain_is_none() {
        let chain = [pr(1, "a", "main"), pr(2, "b", "a"), pr(3, "c", "a")];
        let forked = [fork(1, "a", "main"), pr(2, "b", "a")];
        let apart = [pr(1, "a", "main"), pr(2, "b", "main")];

        assert_eq!(RepoStack::native("o/r", 1, &[1, 2, 3], &chain), None);
        assert_eq!(RepoStack::native("o/r", 1, &[1, 2, 7], &chain), None);
        assert_eq!(RepoStack::native("o/r", 1, &[1], &chain), None);
        assert_eq!(RepoStack::native("o/r", 1, &[1, 2], &forked), None);
        assert_eq!(RepoStack::native("o/r", 1, &[1, 2], &apart), None);
    }

    #[test]
    fn a_base_chain_finds_parent_and_children() {
        let prs = [
            pr(3, "c", "b"),
            pr(2, "b", "a"),
            pr(1, "a", "main"),
            pr(8, "x", "main"),
        ];

        let stack = RepoStack::from_base_chain("o/r", 2, &prs).expect("2 is stacked");

        assert_eq!(entry_numbers(&stack), vec![1, 2, 3]);
        assert_eq!(stack.parent_of(2), Some(1));
        assert_eq!(stack.children_of(2), vec![3]);
        assert_eq!(stack.parent_of(1), None);
        assert!(stack.children_of(3).is_empty());
        assert!(stack.children_of(99).is_empty());
        assert_eq!(stack.trunk(), "main");
        assert_eq!(stack.repo(), "o/r");
        assert_eq!(stack.source(), Source::BaseChain);
        assert_eq!(stack.github_number(), None);
    }

    #[test]
    fn two_children_of_one_pr_are_both_listed() {
        let prs = [pr(3, "c", "a"), pr(2, "b", "a"), pr(1, "a", "main")];

        let stack = RepoStack::from_base_chain("o/r", 3, &prs).expect("3 is stacked");

        assert_eq!(stack.children_of(1), vec![2, 3]);
        assert_eq!(entry_numbers(&stack), vec![1, 2, 3]);
    }

    #[test]
    fn a_cross_fork_or_cyclic_base_is_no_stack() {
        let forked = [fork(1, "a", "main"), pr(2, "b", "a")];
        let looped = [pr(1, "a", "b"), pr(2, "b", "a")];

        assert_eq!(RepoStack::from_base_chain("o/r", 2, &forked), None);
        assert_eq!(RepoStack::from_base_chain("o/r", 1, &looped), None);
        assert_eq!(RepoStack::from_base_chain("o/r", 2, &looped), None);
    }

    #[test]
    fn a_pull_request_alone_is_no_stack() {
        let prs = [pr(1, "a", "main"), pr(2, "b", "main")];

        assert_eq!(RepoStack::from_base_chain("o/r", 1, &prs), None);
        assert_eq!(RepoStack::from_base_chain("o/r", 5, &prs), None);
    }

    #[test]
    fn empty_input_gives_an_empty_stack() {
        let stack = Stack::new(&[]);

        assert!(stack.nodes().is_empty());
        assert!(stack.tree_of(1).is_empty());
        assert!(stack.native_chains().is_empty());
    }
}
