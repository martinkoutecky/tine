#include <wasi/api.h>

// Match Emscripten 3.1's fd_close result in a build without a filesystem.
__wasi_errno_t __wrap___wasi_fd_close(__wasi_fd_t fd) {
    (void)fd;
    return __WASI_ERRNO_NOSYS;
}
