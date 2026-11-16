// Native fixture only. Production account lookup stays getpwuid_r. A Linux
// CI account can have an absent/unwritable passwd home; map its home into the
// disposable run while retaining real account permissions and kernel locks.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <pwd.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

// Each app/CLI exec gets its own private startup cause, including relaunches.
__attribute__((constructor)) static void debug_log_for_launch(void) {
    const char *directory = getenv("TINE_E2E_DEBUG_DIR");
    if (directory) {
        char *file = NULL;
        if (asprintf(&file, "%s/tine-debug-%ld.log", directory, (long)getpid()) >= 0) {
            setenv("TINE_DEBUG_LOG", file, 1);
            free(file);
        }
    }
}

int getpwuid_r(uid_t uid, struct passwd *entry, char *buffer, size_t length,
               struct passwd **result) {
    typedef int (*lookup)(uid_t, struct passwd *, char *, size_t, struct passwd **);
    lookup original = (lookup)dlsym(RTLD_NEXT, "getpwuid_r");
    int status = original(uid, entry, buffer, length, result);
    const char *fixture = getenv("TINE_E2E_ACCOUNT_HOME");
    if (!status && *result && uid == getuid() && fixture) {
        static __thread char home[64];
        static __thread int held = -1;
        // Keep the descriptor for the process lifetime. The /proc alias is
        // short enough for sockaddr_un and names the same directory in every
        // process, including the process claiming the old draft owner's lock.
        if (held < 0) held = open(fixture, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
        if (held < 0) {
            // A broken fixture must never fall back to the real account home.
            int error = errno;
            *result = NULL;
            return error;
        }
        snprintf(home, sizeof(home), "/proc/self/fd/%d", held);
        entry->pw_dir = home;
    }
    return status;
}
