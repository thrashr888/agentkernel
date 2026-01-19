/**
 * Simple HTTP server in C++ for demonstrating agentkernel.
 *
 * Compile: g++ -o server_cpp server.cpp
 * Run: ./server_cpp
 */

#include <iostream>
#include <string>
#include <cstring>
#include <ctime>
#include <sstream>
#include <iomanip>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>

constexpr int PORT = 8080;
constexpr int BUFFER_SIZE = 4096;

std::string get_timestamp() {
    auto now = std::time(nullptr);
    auto tm = *std::gmtime(&now);
    std::ostringstream oss;
    oss << std::put_time(&tm, "%Y-%m-%dT%H:%M:%SZ");
    return oss.str();
}

void handle_request(int client_socket) {
    char buffer[BUFFER_SIZE];

    ssize_t bytes_read = read(client_socket, buffer, BUFFER_SIZE - 1);
    if (bytes_read < 0) {
        std::cerr << "Error reading from socket" << std::endl;
        return;
    }
    buffer[bytes_read] = '\0';
    std::string request(buffer);

    std::string response;
    if (request.find("GET /health") != std::string::npos) {
        response = "HTTP/1.1 200 OK\r\n"
                   "Content-Type: application/json\r\n"
                   "\r\n"
                   R"({"status": "ok", "timestamp": ")" + get_timestamp() + "\"}\n";
    } else {
        response = "HTTP/1.1 200 OK\r\n"
                   "Content-Type: text/plain\r\n"
                   "\r\n"
                   "Hello from agentkernel sandbox!\n";
    }

    write(client_socket, response.c_str(), response.length());
}

int main() {
    int server_socket = socket(AF_INET, SOCK_STREAM, 0);
    if (server_socket < 0) {
        std::cerr << "Failed to create socket" << std::endl;
        return 1;
    }

    int opt = 1;
    setsockopt(server_socket, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));

    sockaddr_in server_addr{};
    server_addr.sin_family = AF_INET;
    server_addr.sin_addr.s_addr = INADDR_ANY;
    server_addr.sin_port = htons(PORT);

    if (bind(server_socket, reinterpret_cast<sockaddr*>(&server_addr), sizeof(server_addr)) < 0) {
        std::cerr << "Bind failed" << std::endl;
        return 1;
    }

    if (listen(server_socket, 5) < 0) {
        std::cerr << "Listen failed" << std::endl;
        return 1;
    }

    std::cout << "C++ server listening on port " << PORT << std::endl;

    while (true) {
        sockaddr_in client_addr{};
        socklen_t client_len = sizeof(client_addr);
        int client_socket = accept(server_socket, reinterpret_cast<sockaddr*>(&client_addr), &client_len);

        if (client_socket < 0) {
            std::cerr << "Accept failed" << std::endl;
            continue;
        }

        handle_request(client_socket);
        close(client_socket);
    }

    close(server_socket);
    return 0;
}
