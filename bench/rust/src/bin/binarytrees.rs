// Binary trees, built and consumed, three repetitions of growing depth. The
// allocation kernel: every node is one heap block in every language, so the
// kernel measures the allocator and the drop path -- malloc/free, a garbage
// collector, or Box's drop glue.
//
// A leaf carries the path tag it was built under, so the checksum is a real
// function of the shape rather than a node count.

const FIRST_DEPTH: i64 = 13;
const REPETITIONS: i64 = 3;

enum Tree {
    Leaf(i64),
    Node(Box<Tree>, Box<Tree>),
}

fn build(depth: i64, tag: i64) -> Tree {
    if depth == 0 {
        Tree::Leaf(tag)
    } else {
        Tree::Node(
            Box::new(build(depth - 1, tag * 2)),
            Box::new(build(depth - 1, tag * 2 + 1)),
        )
    }
}

fn sum_tree(tree: Tree) -> i64 {
    match tree {
        Tree::Leaf(value) => value,
        Tree::Node(left, right) => sum_tree(*left) + sum_tree(*right),
    }
}

fn main() {
    let mut total = 0;
    for rep in 0..REPETITIONS {
        let tree = build(FIRST_DEPTH + rep, 1);
        total += sum_tree(tree);
    }
    println!("{}", total);
}
