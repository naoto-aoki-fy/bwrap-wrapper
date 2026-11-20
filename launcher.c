#define _POSIX_C_SOURCE 200809L

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* With -S, use the tool environment's package directories without running site hooks.
 * Derive the prefix from the interpreter path for Python versions before 3.14. */
static char bootstrap[] =
    "import os, site, sys\n"
    "prefix = os.path.dirname(os.path.dirname(sys.executable))\n"
    "sys.path.extend(site.getsitepackages([prefix]))\n"
    "from bwrap_wrapper.runtime import main\n"
    "main(sys.argv.pop(1))\n";

int main(int argc, char *argv[])
{
    char launcher[PATH_MAX];
    ssize_t length = readlink("/proc/self/exe", launcher, sizeof(launcher));
    if (length < 0) {
        perror("bwrap-wrapper: read /proc/self/exe");
        return 1;
    }
    if ((size_t)length == sizeof(launcher)) {
        fputs("bwrap-wrapper: launcher path is too long\n", stderr);
        return 1;
    }
    launcher[length] = '\0';

    char python[PATH_MAX + sizeof("python")];
    size_t bin_length = (size_t)(strrchr(launcher, '/') - launcher) + 1;
    memcpy(python, launcher, bin_length);
    memcpy(python + bin_length, "python", sizeof("python"));

    char **args = calloc((size_t)argc + 5, sizeof(*args));
    if (args == NULL) {
        perror("bwrap-wrapper: allocate Python arguments");
        return 1;
    }
    args[0] = python;
    args[1] = "-ISBXutf8";
    args[2] = "-c";
    args[3] = bootstrap;
    args[4] = launcher;
    for (int i = 1; i < argc; i++) {
        args[i + 4] = argv[i];
    }

    execv(python, args);
    perror("bwrap-wrapper: execute the tool environment's Python");
    free(args);
    return 1;
}
