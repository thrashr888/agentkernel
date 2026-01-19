/**
 * Simple HTTP server in C for demonstrating agentkernel.
 *
 * Compile: gcc -o server server.c
 * Run: ./server
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <time.h>

#define PORT 8080
#define BUFFER_SIZE 4096

void handle_request(int client_socket) {
    char buffer[BUFFER_SIZE];
    char response[BUFFER_SIZE];

    // Read request
    ssize_t bytes_read = read(client_socket, buffer, BUFFER_SIZE - 1);
    if (bytes_read < 0) {
        perror("read");
        return;
    }
    buffer[bytes_read] = '\0';

    // Check for /health endpoint
    if (strstr(buffer, "GET /health") != NULL) {
        time_t now = time(NULL);
        char timestamp[64];
        strftime(timestamp, sizeof(timestamp), "%Y-%m-%dT%H:%M:%SZ", gmtime(&now));

        snprintf(response, sizeof(response),
            "HTTP/1.1 200 OK\r\n"
            "Content-Type: application/json\r\n"
            "\r\n"
            "{\"status\": \"ok\", \"timestamp\": \"%s\"}\n",
            timestamp);
    } else {
        snprintf(response, sizeof(response),
            "HTTP/1.1 200 OK\r\n"
            "Content-Type: text/plain\r\n"
            "\r\n"
            "Hello from agentkernel sandbox!\n");
    }

    write(client_socket, response, strlen(response));
}

int main() {
    int server_socket, client_socket;
    struct sockaddr_in server_addr, client_addr;
    socklen_t client_len = sizeof(client_addr);

    // Create socket
    server_socket = socket(AF_INET, SOCK_STREAM, 0);
    if (server_socket < 0) {
        perror("socket");
        exit(1);
    }

    // Allow address reuse
    int opt = 1;
    setsockopt(server_socket, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

    // Bind
    memset(&server_addr, 0, sizeof(server_addr));
    server_addr.sin_family = AF_INET;
    server_addr.sin_addr.s_addr = INADDR_ANY;
    server_addr.sin_port = htons(PORT);

    if (bind(server_socket, (struct sockaddr*)&server_addr, sizeof(server_addr)) < 0) {
        perror("bind");
        exit(1);
    }

    // Listen
    if (listen(server_socket, 5) < 0) {
        perror("listen");
        exit(1);
    }

    printf("C server listening on port %d\n", PORT);

    // Accept connections
    while (1) {
        client_socket = accept(server_socket, (struct sockaddr*)&client_addr, &client_len);
        if (client_socket < 0) {
            perror("accept");
            continue;
        }

        handle_request(client_socket);
        close(client_socket);
    }

    close(server_socket);
    return 0;
}
