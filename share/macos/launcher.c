// LaunchServices must start native code inside the bundle so TCC can attribute
// the process to nekomata rather than to the shell interpreting a wrapper.
#include <errno.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void fail(const char *operation) {
    perror(operation);
    exit(1);
}

static void prefix(const char *name, const char *value, const char *fallback) {
    const char *old = getenv(name);
    if (!old) old = fallback;
    char *combined = NULL;
    if (asprintf(&combined, "%s%s%s", value, old && *old ? ":" : "", old ? old : "") < 0)
        fail("allocate environment");
    if (setenv(name, combined, 1) != 0) fail(name);
    free(combined);
}

int main(int argc, char **argv) {
    (void)argc;
    char path[PATH_MAX];
    uint32_t size = sizeof(path);
    if (_NSGetExecutablePath(path, &size) != 0) {
        fprintf(stderr, "nekomata launcher path is too long\n");
        return 1;
    }
    char *executable = realpath(path, NULL);
    if (!executable) fail("resolve launcher");
    char *separator = strrchr(executable, '/');
    if (!separator) { free(executable); return 1; }
    *separator = '\0';
    char *binary = NULL;
    if (asprintf(&binary, "%s/nekomata-bin", executable) < 0) fail("allocate executable path");
    free(executable);

    prefix("PATH", "@package@/bin:@felis@/bin", "/usr/bin:/bin:/usr/sbin:/sbin");
    prefix("XDG_DATA_DIRS", "@package@/share", "/usr/local/share:/usr/share");
    const char *home = getenv("HOME");
    if (!home || chdir(home) != 0) {
        fprintf(stderr, "nekomata could not open the home directory\n");
        return 1;
    }
    argv[0] = binary;
    execv(binary, argv);
    fail("launch nekomata");
}
