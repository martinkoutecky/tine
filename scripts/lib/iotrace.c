// iotrace: a small ptrace syscall tracer for the storage A/B bench's unit-cost
// measurement (scripts/bench-storage-ab.mjs). strace is not installed on the
// bench machine, so this records the file-modifying syscalls of ONE running
// process (all its threads; child processes are not followed) as NDJSON.
//
//   iotrace <pid> <out.ndjson> <path-prefix>...
//
// One line per completed syscall whose resolved path (or fd path) starts with a
// prefix: {"t":<CLOCK_REALTIME ms>,"tid":N,"sys":"write","path":"...",
// "path2":"...","n":<bytes or 0>,"ret":R,"isdir":0|1,"flags":F}. It prints
// "ready" on stdout once every thread is attached and resumed, and detaches
// cleanly on SIGTERM/SIGINT (flushing the file) or exits when the tracee does.
// x86_64 Linux only (syscall numbers); see the A/B harness for how events are
// attributed to a single edit. Not a security boundary: it observes only.
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/uio.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#ifndef __x86_64__
#error "iotrace supports x86_64 only"
#endif

// <linux/ptrace.h> clashes with <sys/ptrace.h>, so the (stable, kernel >= 5.3)
// PTRACE_GET_SYSCALL_INFO ABI is restated here under private names.
#define IOT_GET_SYSCALL_INFO 0x420e
#define IOT_INFO_ENTRY 1
#define IOT_INFO_EXIT 2
struct iot_syscall_info {
  uint8_t op;
  uint8_t pad[3];
  uint32_t arch;
  uint64_t instruction_pointer;
  uint64_t stack_pointer;
  union {
    struct { uint64_t nr; uint64_t args[6]; } entry;
    struct { int64_t rval; uint8_t is_error; } exit;
    struct { uint64_t nr; uint64_t args[6]; uint32_t ret_data; } seccomp;
  };
};

#define MAXT 4096
#define MAXP 16

typedef struct {
  pid_t tid;
  int used;
  int detached;
  int tracked;       // entry captured and passed the prefix filter
  long nr;
  uint64_t args[6];
  char path[PATH_MAX];
  char path2[PATH_MAX];
  int isdir;
} Thread;

static Thread threads[MAXT];
static const char *prefixes[MAXP];
static int nprefix;
static FILE *out;
static volatile sig_atomic_t stop_requested;

static void on_signal(int s) { (void)s; stop_requested = 1; }

static Thread *find(pid_t tid) {
  for (int i = 0; i < MAXT; i++)
    if (threads[i].used && threads[i].tid == tid) return &threads[i];
  return NULL;
}

static Thread *add(pid_t tid) {
  Thread *t = find(tid);
  if (t) return t;
  for (int i = 0; i < MAXT; i++)
    if (!threads[i].used) {
      memset(&threads[i], 0, sizeof threads[i]);
      threads[i].used = 1;
      threads[i].tid = tid;
      return &threads[i];
    }
  return NULL;
}

static double now_ms(void) {
  struct timespec ts;
  clock_gettime(CLOCK_REALTIME, &ts);
  return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1e6;
}

static int read_tracee_string(pid_t tid, uint64_t addr, char *buf, size_t cap) {
  if (!addr) { buf[0] = 0; return 0; }
  size_t got = 0;
  while (got + 1 < cap) {
    size_t want = cap - 1 - got;
    if (want > 256) want = 256;
    // Do not cross a page boundary in one read.
    size_t to_page = 4096 - ((addr + got) & 4095);
    if (want > to_page) want = to_page;
    struct iovec local = {buf + got, want}, remote = {(void *)(addr + got), want};
    ssize_t n = process_vm_readv(tid, &local, 1, &remote, 1, 0);
    if (n <= 0) { buf[got] = 0; return got > 0 ? 0 : -1; }
    for (ssize_t i = 0; i < n; i++)
      if (buf[got + i] == 0) return 0;
    got += (size_t)n;
  }
  buf[cap - 1] = 0;
  return 0;
}

static void fd_path(pid_t tid, long fd, char *buf, size_t cap) {
  char link[64];
  snprintf(link, sizeof link, "/proc/%d/fd/%ld", tid, fd);
  ssize_t n = readlink(link, buf, cap - 1);
  buf[n > 0 ? n : 0] = 0;
}

// Resolve a (dirfd, user pointer) pair to an absolute path in `buf`.
static void path_at(pid_t tid, long dirfd, uint64_t ptr, char *buf, size_t cap) {
  char raw[PATH_MAX];
  buf[0] = 0;
  if (read_tracee_string(tid, ptr, raw, sizeof raw) != 0) return;
  if (raw[0] == '/') { snprintf(buf, cap, "%s", raw); return; }
  char base[PATH_MAX];
  base[0] = 0;
  if ((int)dirfd == AT_FDCWD) {
    char link[64];
    snprintf(link, sizeof link, "/proc/%d/cwd", tid);
    ssize_t n = readlink(link, base, sizeof base - 1);
    base[n > 0 ? n : 0] = 0;
  } else {
    fd_path(tid, dirfd, base, sizeof base);
  }
  snprintf(buf, cap, "%s/%s", base, raw);
}

static int matches(const char *p) {
  if (!p[0]) return 0;
  for (int i = 0; i < nprefix; i++)
    if (strncmp(p, prefixes[i], strlen(prefixes[i])) == 0) return 1;
  return 0;
}

static const char *sysname(long nr) {
  switch (nr) {
    case SYS_open: return "open";
    case SYS_openat: return "openat";
    case SYS_creat: return "creat";
    case SYS_write: return "write";
    case SYS_pwrite64: return "pwrite64";
    case SYS_writev: return "writev";
    case SYS_pwritev: return "pwritev";
    case SYS_pwritev2: return "pwritev2";
    case SYS_fsync: return "fsync";
    case SYS_fdatasync: return "fdatasync";
    case SYS_sync_file_range: return "sync_file_range";
    case SYS_syncfs: return "syncfs";
    case SYS_ftruncate: return "ftruncate";
    case SYS_truncate: return "truncate";
    case SYS_fallocate: return "fallocate";
    case SYS_rename: return "rename";
    case SYS_renameat: return "renameat";
    case SYS_renameat2: return "renameat2";
    case SYS_unlink: return "unlink";
    case SYS_unlinkat: return "unlinkat";
    case SYS_mkdir: return "mkdir";
    case SYS_mkdirat: return "mkdirat";
    case SYS_rmdir: return "rmdir";
    case SYS_link: return "link";
    case SYS_linkat: return "linkat";
    case SYS_copy_file_range: return "copy_file_range";
    case SYS_sendfile: return "sendfile";
    default: return NULL;
  }
}

// Capture what the syscall names at entry (memory and fds are still valid).
static void capture(Thread *t, pid_t tid) {
  t->path[0] = 0;
  t->path2[0] = 0;
  t->isdir = 0;
  const uint64_t *a = t->args;
  switch (t->nr) {
    case SYS_open: case SYS_creat: case SYS_truncate: case SYS_unlink:
    case SYS_mkdir: case SYS_rmdir:
      path_at(tid, AT_FDCWD, a[0], t->path, sizeof t->path); break;
    case SYS_openat: case SYS_unlinkat: case SYS_mkdirat:
      path_at(tid, (long)a[0], a[1], t->path, sizeof t->path); break;
    case SYS_rename: case SYS_link:
      path_at(tid, AT_FDCWD, a[0], t->path, sizeof t->path);
      path_at(tid, AT_FDCWD, a[1], t->path2, sizeof t->path2); break;
    case SYS_renameat: case SYS_renameat2: case SYS_linkat:
      path_at(tid, (long)a[0], a[1], t->path, sizeof t->path);
      path_at(tid, (long)a[2], a[3], t->path2, sizeof t->path2); break;
    case SYS_write: case SYS_pwrite64: case SYS_writev: case SYS_pwritev:
    case SYS_pwritev2: case SYS_fsync: case SYS_fdatasync:
    case SYS_sync_file_range: case SYS_syncfs: case SYS_ftruncate:
    case SYS_fallocate: case SYS_sendfile:
      fd_path(tid, (long)a[0], t->path, sizeof t->path); break;
    case SYS_copy_file_range:
      fd_path(tid, (long)a[2], t->path, sizeof t->path); break;
    default: break;
  }
  if (t->path[0] && (t->nr == SYS_fsync || t->nr == SYS_fdatasync || t->nr == SYS_syncfs)) {
    struct stat st;
    if (stat(t->path, &st) == 0 && S_ISDIR(st.st_mode)) t->isdir = 1;
  }
  t->tracked = matches(t->path) || matches(t->path2);
}

static void json_str(FILE *f, const char *s) {
  fputc('"', f);
  for (; *s; s++) {
    unsigned char c = (unsigned char)*s;
    if (c == '"' || c == '\\') { fputc('\\', f); fputc(c, f); }
    else if (c < 0x20) fprintf(f, "\\u%04x", c);
    else fputc(c, f);
  }
  fputc('"', f);
}

static void emit(Thread *t, long ret) {
  const char *name = sysname(t->nr);
  if (!name) return;
  long n = 0;
  switch (t->nr) {
    case SYS_write: case SYS_pwrite64: case SYS_writev: case SYS_pwritev:
    case SYS_pwritev2: case SYS_sendfile: case SYS_copy_file_range:
      n = ret > 0 ? ret : 0; break;
    default: break;
  }
  uint64_t flags = 0;
  if (t->nr == SYS_open) flags = t->args[1];
  else if (t->nr == SYS_openat) flags = t->args[2];
  else if (t->nr == SYS_creat) flags = O_CREAT;
  else if (t->nr == SYS_renameat2) flags = t->args[4];
  fprintf(out, "{\"t\":%.3f,\"tid\":%d,\"sys\":\"%s\",\"path\":", now_ms(), t->tid, name);
  json_str(out, t->path);
  fprintf(out, ",\"path2\":");
  json_str(out, t->path2);
  fprintf(out, ",\"n\":%ld,\"ret\":%ld,\"isdir\":%d,\"flags\":%llu}\n", n, ret, t->isdir,
          (unsigned long long)flags);
}

static void resume(pid_t tid, int sig) { ptrace(PTRACE_SYSCALL, tid, 0, (void *)(long)sig); }

static void on_syscall_stop(Thread *t, pid_t tid) {
  struct iot_syscall_info info;
  memset(&info, 0, sizeof info);
  if (ptrace(IOT_GET_SYSCALL_INFO, tid, (void *)sizeof info, &info) < 0) return;
  if (info.op == IOT_INFO_ENTRY) {
    t->nr = (long)info.entry.nr;
    memcpy(t->args, info.entry.args, sizeof t->args);
    t->tracked = 0;
    if (sysname(t->nr)) {
      capture(t, tid);
      if (t->nr == SYS_syncfs) t->tracked = 1;
    } else {
      t->nr = -1;
    }
  } else if (info.op == IOT_INFO_EXIT) {
    if (t->nr >= 0 && t->tracked) emit(t, (long)info.exit.rval);
    t->tracked = 0;
    t->nr = -1;
  }
}

int main(int argc, char **argv) {
  if (argc < 4) {
    fprintf(stderr, "usage: iotrace <pid> <out.ndjson> <path-prefix>...\n");
    return 2;
  }
  pid_t target = (pid_t)atoi(argv[1]);
  out = fopen(argv[2], "w");
  if (!out) { perror("open out"); return 2; }
  for (int i = 3; i < argc && nprefix < MAXP; i++) prefixes[nprefix++] = argv[i];

  struct sigaction sa;
  memset(&sa, 0, sizeof sa);
  sa.sa_handler = on_signal;
  sigaction(SIGTERM, &sa, NULL);
  sigaction(SIGINT, &sa, NULL);

  char dir[64];
  snprintf(dir, sizeof dir, "/proc/%d/task", target);
  DIR *d = opendir(dir);
  if (!d) { perror("opendir task"); return 2; }
  struct dirent *e;
  int attached = 0;
  while ((e = readdir(d))) {
    pid_t tid = (pid_t)atoi(e->d_name);
    if (tid <= 0) continue;
    if (ptrace(PTRACE_SEIZE, tid, 0,
               (void *)(long)(PTRACE_O_TRACESYSGOOD | PTRACE_O_TRACECLONE)) < 0) {
      fprintf(stderr, "seize %d: %s\n", tid, strerror(errno));
      continue;
    }
    Thread *t = add(tid);
    if (t) t->nr = -1;
    ptrace(PTRACE_INTERRUPT, tid, 0, 0);
    attached++;
  }
  closedir(d);
  if (!attached) { fprintf(stderr, "iotrace: attached to no thread\n"); return 2; }

  int live = attached, interrupted = 0, ready_printed = 0;
  int pending_first_stops = attached;
  while (live > 0) {
    if (stop_requested && !interrupted) {
      interrupted = 1;
      for (int i = 0; i < MAXT; i++)
        if (threads[i].used && !threads[i].detached) ptrace(PTRACE_INTERRUPT, threads[i].tid, 0, 0);
    }
    int st;
    pid_t tid = waitpid(-1, &st, __WALL);
    if (tid < 0) {
      if (errno == EINTR) continue;
      break;
    }
    Thread *t = find(tid);
    int fresh = 0;
    if (!t) { t = add(tid); if (t) t->nr = -1; fresh = 1; live++; }
    if (!t) continue;
    if (WIFEXITED(st) || WIFSIGNALED(st)) {
      t->used = 0;
      live--;
      continue;
    }
    if (!WIFSTOPPED(st)) continue;
    int sig = WSTOPSIG(st);
    int event = (st >> 16) & 0xff;

    if (interrupted && !t->detached) {
      ptrace(PTRACE_DETACH, tid, 0, 0);
      t->detached = 1;
      t->used = 0;
      live--;
      continue;
    }
    if (sig == (SIGTRAP | 0x80)) {
      on_syscall_stop(t, tid);
      resume(tid, 0);
    } else if (event == PTRACE_EVENT_CLONE) {
      resume(tid, 0);  // the new thread reports its own initial stop and is added there
    } else if (event == PTRACE_EVENT_STOP) {
      // Our own PTRACE_INTERRUPT (first stop after SEIZE) or a group stop.
      if (pending_first_stops > 0 && !fresh) {
        pending_first_stops--;
        if (pending_first_stops == 0 && !ready_printed) {
          ready_printed = 1;
          printf("ready\n");
          fflush(stdout);
        }
      }
      resume(tid, 0);
    } else if (fresh && sig == SIGSTOP) {
      resume(tid, 0);  // a new thread's initial stop
    } else {
      resume(tid, sig);  // a real signal in flight: deliver it unchanged
    }
    fflush(out);
  }
  fflush(out);
  fclose(out);
  return 0;
}
