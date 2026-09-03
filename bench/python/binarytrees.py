# Binary trees, built and consumed, three repetitions of growing depth. The
# allocation kernel: every node is one heap object in every language, so the
# kernel measures the allocator and the garbage collector.
#
# A leaf carries the path tag it was built under, so the checksum is a real
# function of the shape rather than a node count.

FIRST_DEPTH = 13
REPETITIONS = 3


class Node:
    __slots__ = ("left", "right")

    def __init__(self, left, right):
        self.left = left
        self.right = right


def build(depth, tag):
    if depth == 0:
        return tag
    return Node(build(depth - 1, tag * 2), build(depth - 1, tag * 2 + 1))


def sum_tree(tree):
    if isinstance(tree, Node):
        return sum_tree(tree.left) + sum_tree(tree.right)
    return tree


total = 0
for rep in range(REPETITIONS):
    total += sum_tree(build(FIRST_DEPTH + rep, 1))

print(total)
