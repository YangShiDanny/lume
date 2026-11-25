#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#pragma comment(lib, "ws2_32.lib")
#define LUME_TEST_CLOSE closesocket
typedef SOCKET lume_test_socket;
#define LUME_TEST_INVALID INVALID_SOCKET
#define LUME_TEST_IS_INVALID(fd) ((fd) == INVALID_SOCKET)
static void lume_test_sleep(void) { Sleep(100); }
#else
#include <arpa/inet.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
#define LUME_TEST_CLOSE close
typedef int lume_test_socket;
#define LUME_TEST_INVALID (-1)
#define LUME_TEST_IS_INVALID(fd) ((fd) < 0)
static void lume_test_sleep(void) { usleep(100000); }
#endif

static lume_test_socket connect_port(uint16_t port) {
    for (int attempt = 0; attempt < 50; ++attempt) {
        lume_test_socket fd = (lume_test_socket)socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in address;
        memset(&address, 0, sizeof(address));
        address.sin_family = AF_INET;
        address.sin_port = htons(port);
        address.sin_addr.s_addr = inet_addr("127.0.0.1");
        if (connect(fd, (const struct sockaddr *)&address, sizeof(address)) == 0) {
#ifdef _WIN32
            DWORD timeout = 3000;
#else
            struct timeval timeout = {3, 0};
#endif
            setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO,
                       (const char *)&timeout, sizeof(timeout));
            return fd;
        }
        LUME_TEST_CLOSE(fd);
        lume_test_sleep();
    }
    return LUME_TEST_INVALID;
}

static int recv_exact(lume_test_socket fd, char *buffer, int length) {
    int total = 0;
    while (total < length) {
        int got = (int)recv(fd, buffer + total, (size_t)(length - total), 0);
        if (got <= 0) {
            return -1;
        }
        total += got;
    }
    return total;
}

static int exchange(lume_test_socket fd, const char *message) {
    int length = (int)strlen(message);
    if (send(fd, message, (size_t)length, 0) != length) {
        return 0;
    }
    char buffer[256] = {0};
    int received = recv_exact(fd, buffer, length);
    if (received != length || memcmp(buffer, message, (size_t)length) != 0) {
        fprintf(stderr, "bad echo: expected %s, got %s\n", message, buffer);
        return 0;
    }
    return 1;
}

int main(int argc, char **argv) {
    uint16_t port = argc > 1 ? (uint16_t)atoi(argv[1]) : 18080;
#ifdef _WIN32
    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        return 1;
    }
#endif

    lume_test_socket first = connect_port(port);
    if (LUME_TEST_IS_INVALID(first)) {
        fprintf(stderr, "echo server did not start on port %u\n", (unsigned)port);
        return 1;
    }
    if (!exchange(first, "client-a-first")) {
        return 1;
    }

    lume_test_socket second = connect_port(port);
    if (LUME_TEST_IS_INVALID(second)) {
        return 1;
    }
    if (!exchange(second, "client-b-first")) {
        return 1;
    }

    /* The first client is still open: a sequential accept/echo loop would be
       stuck serving it and could never answer this second request. */
    if (!exchange(first, "client-a-second")) {
        return 1;
    }

    LUME_TEST_CLOSE(first);
    LUME_TEST_CLOSE(second);
    printf("ECHO_MULTI_OK\n");
    return 0;
}
