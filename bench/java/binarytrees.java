// Binary trees, built and consumed, three repetitions of growing depth. The
// allocation kernel: every node is one heap object in every language, so the
// kernel measures the allocator and the garbage collector.
//
// A leaf carries the path tag it was built under, so the checksum is a real
// function of the shape rather than a node count.

public class binarytrees {
    static final int FIRST_DEPTH = 13;
    static final int REPETITIONS = 3;

    static class Tree {
        Tree left;  // null for a leaf
        Tree right; // null for a leaf
        long value; // path tag, meaningful for a leaf
    }

    static Tree build(int depth, long tag) {
        Tree node = new Tree();
        if (depth == 0) {
            node.value = tag;
            return node;
        }
        node.left = build(depth - 1, tag * 2);
        node.right = build(depth - 1, tag * 2 + 1);
        return node;
    }

    static long sumTree(Tree tree) {
        if (tree.left == null)
            return tree.value;
        return sumTree(tree.left) + sumTree(tree.right);
    }

    public static void main(String[] args) {
        long total = 0;
        for (int rep = 0; rep < REPETITIONS; rep++) {
            total += sumTree(build(FIRST_DEPTH + rep, 1));
        }
        System.out.println(total);
    }
}
