/* A resource with no destructor, which is the whole reason `defer` exists
 * (`D-084`, `D-133`): the handle is an `int64_t` and the language cannot run
 * anything when one dies.
 *
 * The counters are what the fixture asserts against. A deferred close that ran
 * twice, or not at all, is a different pair of numbers. */

#include <stdint.h>

static int64_t opened = 0;
static int64_t closed = 0;

int64_t handle_open(void) {
    opened += 1;
    return opened;
}

void handle_close(int64_t handle) {
    (void)handle;
    closed += 1;
}

int64_t handle_opened(void) { return opened; }

int64_t handle_closed(void) { return closed; }
