/* Binary trees, built and consumed, three repetitions of growing depth. The
 * allocation kernel: every node is one heap block in every language, so the
 * kernel measures the allocator and the free path.
 *
 * A leaf carries the path tag it was built under, so the checksum is a real
 * function of the shape rather than a node count. */

#include <stdio.h>
#include <stdlib.h>

#define FIRST_DEPTH 13
#define REPETITIONS 3

typedef struct Tree
{
    struct Tree *left;  /* NULL for a leaf */
    struct Tree *right; /* NULL for a leaf */
    long long value;    /* path tag, meaningful for a leaf */
} Tree;

static Tree *build(int depth, long long tag)
{
    Tree *node = malloc(sizeof(Tree));
    if (depth == 0)
    {
        node->left = NULL;
        node->right = NULL;
        node->value = tag;
        return node;
    }
    node->left = build(depth - 1, tag * 2);
    node->right = build(depth - 1, tag * 2 + 1);
    node->value = 0;
    return node;
}

static long long sum_tree(Tree *tree)
{
    if (tree->left == NULL)
        return tree->value;
    return sum_tree(tree->left) + sum_tree(tree->right);
}

static void free_tree(Tree *tree)
{
    if (tree->left != NULL)
    {
        free_tree(tree->left);
        free_tree(tree->right);
    }
    free(tree);
}

int main(void)
{
    long long total = 0;
    for (int rep = 0; rep < REPETITIONS; rep++)
    {
        Tree *tree = build(FIRST_DEPTH + rep, 1);
        total += sum_tree(tree);
        free_tree(tree);
    }
    printf("%lld\n", total);
    return 0;
}
