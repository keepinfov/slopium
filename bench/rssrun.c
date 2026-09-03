/* The measuring wrapper the harness spawns instead of the program itself.
 *
 * A Python harness cannot measure a child's peak RSS honestly: every spawn
 * primitive it has (posix_spawn's vfork, or a plain fork's copy-on-write
 * window) makes the child inherit the interpreter's resident pages before
 * exec, and ru_maxrss keeps that peak. This file is small on purpose -- its
 * own footprint is the floor under the measurement -- and it does the one
 * thing the harness cannot: fork, exec, wait, and report.
 *
 * The child's wall time and peak RSS go to file descriptor 3 as one line,
 * `BENCH <rss_kb> <wall_ms>`, because descriptors 1 and 2 belong to the
 * program under test. The child's exit status is this program's. */

#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <sys/resource.h>
#include <sys/wait.h>

int main(int argc, char **argv)
{
    if (argc < 2)
        return 2;

    struct timespec started;
    struct timespec ended;
    clock_gettime(CLOCK_MONOTONIC, &started);

    pid_t pid = fork();
    if (pid < 0)
        return 2;
    if (pid == 0)
    {
        execvp(argv[1], argv + 1);
        _exit(127);
    }

    int status = 0;
    struct rusage usage;
    if (wait4(pid, &status, 0, &usage) < 0)
        return 2;

    clock_gettime(CLOCK_MONOTONIC, &ended);
    double milliseconds =
        (ended.tv_sec - started.tv_sec) * 1000.0
        + (ended.tv_nsec - started.tv_nsec) / 1000000.0;

    dprintf(3, "BENCH %ld %.3f\n", usage.ru_maxrss, milliseconds);

    if (WIFEXITED(status))
        return WEXITSTATUS(status);
    return 128 + WTERMSIG(status);
}
