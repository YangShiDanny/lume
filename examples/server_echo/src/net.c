#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#pragma comment(lib, "ws2_32.lib")
#define LUME_NET_CLOSE closesocket
typedef SOCKET lume_socket;
typedef WSAPOLLFD lume_pollfd;
#define LUME_NET_POLL_IN POLLIN
static int lume_poll(lume_pollfd *fds, int count, int timeout_ms) {
    return WSAPoll(fds, (ULONG)count, timeout_ms);
}
static int lume_net_error_code(void) { return WSAGetLastError(); }
#else
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#define LUME_NET_CLOSE close
typedef int lume_socket;
typedef struct pollfd lume_pollfd;
#define LUME_NET_POLL_IN POLLIN
static int lume_poll(lume_pollfd *fds, int count, int timeout_ms) {
    return poll(fds, (nfds_t)count, timeout_ms);
}
static int lume_net_error_code(void) { return errno; }
#endif

#define LUME_NET_ERROR (-1)
#define LUME_NET_TIMEOUT (-2)
#define LUME_NET_MAX_FDS 64

static char lume_last_error[256] = "ok";

static void lume_set_error(const char *operation, int code) {
    char detail[128];
    snprintf(detail, sizeof(detail), "%s failed (code %d)", operation, code);
    snprintf(lume_last_error, sizeof(lume_last_error), "%s", detail);
}

static int lume_wait_readable(int64_t fd, int64_t timeout_ms) {
    lume_pollfd entry;
    entry.fd = (lume_socket)fd;
    entry.events = LUME_NET_POLL_IN;
    entry.revents = 0;
    int ready = lume_poll(&entry, 1, (int)timeout_ms);
    if (ready == 0) {
        return LUME_NET_TIMEOUT;
    }
    if (ready < 0) {
        lume_set_error("poll", lume_net_error_code());
        return LUME_NET_ERROR;
    }
#ifdef POLLNVAL
    if (entry.revents & POLLNVAL) {
        lume_set_error("poll", 10001);
        return LUME_NET_ERROR;
    }
#endif
    return 1;
}

static lume_pollfd lume_poll_entries[LUME_NET_MAX_FDS];

static void lume_pollset_reset(int index) {
#ifdef _WIN32
    lume_poll_entries[index].fd = INVALID_SOCKET;
#else
    lume_poll_entries[index].fd = -1;
#endif
    lume_poll_entries[index].events = LUME_NET_POLL_IN;
    lume_poll_entries[index].revents = 0;
}

int64_t lume_net_pollset_clear(void) {
    for (int index = 0; index < LUME_NET_MAX_FDS; ++index) {
        lume_pollset_reset(index);
    }
    return 0;
}

int64_t lume_net_pollset_set(int64_t index, int64_t fd) {
    if (index < 0 || index >= LUME_NET_MAX_FDS) {
        lume_set_error("pollset index", 10004);
        return LUME_NET_ERROR;
    }
    lume_poll_entries[index].fd = (lume_socket)fd;
    lume_poll_entries[index].events = LUME_NET_POLL_IN;
    lume_poll_entries[index].revents = 0;
    return 0;
}

int64_t lume_net_poll(int64_t timeout_ms) {
    int ready = lume_poll(lume_poll_entries, LUME_NET_MAX_FDS, (int)timeout_ms);
    if (ready == 0) {
        return 0;
    }
    if (ready < 0) {
        lume_set_error("poll", lume_net_error_code());
        return LUME_NET_ERROR;
    }
    return (int64_t)ready;
}

int64_t lume_net_pollset_ready(int64_t index) {
    if (index < 0 || index >= LUME_NET_MAX_FDS) {
        lume_set_error("pollset index", 10004);
        return LUME_NET_ERROR;
    }
    short revents = lume_poll_entries[index].revents;
    if (revents == 0) {
        return 0;
    }
    if (revents & LUME_NET_POLL_IN) {
        return 1;
    }
    return LUME_NET_ERROR;
}

int64_t lume_net_listen(int64_t port) {
#ifdef _WIN32
    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        lume_set_error("WSAStartup", lume_net_error_code());
        return LUME_NET_ERROR;
    }
#endif
    lume_socket fd = socket(AF_INET, SOCK_STREAM, 0);
#ifdef _WIN32
    if (fd == INVALID_SOCKET) {
#else
    if (fd < 0) {
#endif
        lume_set_error("socket", lume_net_error_code());
        return LUME_NET_ERROR;
    }
    int reuse = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, (const char *)&reuse, sizeof(reuse));

    struct sockaddr_in address;
    memset(&address, 0, sizeof(address));
    address.sin_family = AF_INET;
    address.sin_port = htons((uint16_t)port);
    address.sin_addr.s_addr = htonl(INADDR_ANY);

    int address_length = (int)sizeof(address);
    if (bind(fd, (const struct sockaddr *)&address, address_length) < 0) {
        lume_set_error("bind", lume_net_error_code());
        LUME_NET_CLOSE(fd);
        return LUME_NET_ERROR;
    }
    if (listen(fd, 64) < 0) {
        lume_set_error("listen", lume_net_error_code());
        LUME_NET_CLOSE(fd);
        return LUME_NET_ERROR;
    }
    strncpy(lume_last_error, "ok", sizeof(lume_last_error) - 1);
    lume_last_error[sizeof(lume_last_error) - 1] = '\0';
    return (int64_t)fd;
}

int64_t lume_net_accept(int64_t server, int64_t timeout_ms) {
    int ready = lume_wait_readable(server, timeout_ms);
    if (ready != 1) {
        return ready;
    }
    lume_socket client = accept((lume_socket)server, NULL, NULL);
#ifdef _WIN32
    if (client == INVALID_SOCKET) {
#else
    if (client < 0) {
#endif
        lume_set_error("accept", lume_net_error_code());
        return LUME_NET_ERROR;
    }
    return (int64_t)client;
}

int64_t lume_net_recv(int64_t client, void *buffer, int64_t length,
                      int64_t timeout_ms) {
    if (buffer == NULL || length <= 0) {
        lume_set_error("recv", 10002);
        return LUME_NET_ERROR;
    }
    int ready = lume_wait_readable(client, timeout_ms);
    if (ready != 1) {
        return ready;
    }
    int received = (int)recv((lume_socket)client, (char *)buffer, (size_t)length, 0);
    if (received < 0) {
        lume_set_error("recv", lume_net_error_code());
        return LUME_NET_ERROR;
    }
    return (int64_t)received;
}

int64_t lume_net_send(int64_t client, const void *buffer, int64_t length) {
    if (buffer == NULL || length < 0) {
        lume_set_error("send", 10003);
        return LUME_NET_ERROR;
    }
    const char *cursor = (const char *)buffer;
    int64_t remaining = length;
    while (remaining > 0) {
        int sent = (int)send((lume_socket)client, cursor, (size_t)remaining, 0);
        if (sent <= 0) {
            lume_set_error("send", lume_net_error_code());
            return LUME_NET_ERROR;
        }
        cursor += sent;
        remaining -= sent;
    }
    return length;
}

void lume_net_close(int64_t fd) { LUME_NET_CLOSE((lume_socket)fd); }

void lume_net_last_error(char *buffer, int64_t length) {
    if (buffer == NULL || length <= 0) {
        return;
    }
    strncpy(buffer, lume_last_error, (size_t)length - 1);
    buffer[length - 1] = '\0';
}
