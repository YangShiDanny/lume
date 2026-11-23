#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#pragma comment(lib, "ws2_32.lib")
#define LUME_TEST_CLOSE closesocket
static void lume_test_sleep(void) { Sleep(100); }
#else
#include <arpa/inet.h>
#include <sys/socket.h>
#include <unistd.h>
#define LUME_TEST_CLOSE close
static void lume_test_sleep(void) { usleep(100000); }
#endif

static const char *MESSAGE = "hello lume";

int main(int argc, char **argv) {
    uint16_t port = argc > 1 ? (uint16_t)atoi(argv[1]) : 18080;
#ifdef _WIN32
    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        return 1;
    }
#endif

    int connected = 0;
    for (int attempt = 0; attempt < 50; ++attempt) {
        int fd = (int)socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in address;
        memset(&address, 0, sizeof(address));
        address.sin_family = AF_INET;
        address.sin_port = htons(port);
        address.sin_addr.s_addr = inet_addr("127.0.0.1");
#ifdef _WIN32
        int result = connect((SOCKET)fd, (const struct sockaddr *)&address, sizeof(address));
#else
        int result = connect(fd, (const struct sockaddr *)&address, sizeof(address));
#endif
        if (result == 0) {
            connected = fd;
            break;
        }
        LUME_TEST_CLOSE((intptr_t)fd);
        lume_test_sleep();
    }
    if (!connected) {
        fprintf(stderr, "echo server did not start on port %u\n", (unsigned)port);
        return 1;
    }

    int fd = connected;
    const char *message = MESSAGE;
    int length = (int)strlen(message);
#ifdef _WIN32
    send((SOCKET)fd, message, length, 0);
#else
    send(fd, message, length, 0);
#endif

    char buffer[256] = {0};
#ifdef _WIN32
    int received = recv((SOCKET)fd, buffer, sizeof(buffer), 0);
#else
    int received = recv(fd, buffer, sizeof(buffer), 0);
#endif
    LUME_TEST_CLOSE((intptr_t)fd);

    if (received != length || memcmp(buffer, message, (size_t)length) != 0) {
        fprintf(stderr, "bad echo: expected %s, got %s\n", message, buffer);
        return 1;
    }
    printf("ECHO_OK %s\n", buffer);
    return 0;
}
