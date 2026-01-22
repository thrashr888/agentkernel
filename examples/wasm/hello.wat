;; Hello World in WebAssembly
;; Demonstrates basic Wasm module structure for Hyperlight

(module
  ;; Import the fd_write function from WASI for console output
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))

  ;; Memory export (required by WASI)
  (memory (export "memory") 1)

  ;; Store the message in memory at offset 8
  (data (i32.const 8) "Hello from Wasm!\n")

  ;; iov structure at offset 0: pointer to string (8), length (17)
  (data (i32.const 0) "\08\00\00\00")  ;; iov.buf = 8
  (data (i32.const 4) "\11\00\00\00")  ;; iov.buf_len = 17

  ;; Main entry point
  (func (export "_start")
    ;; Call fd_write(stdout=1, iovs=0, iovs_len=1, nwritten=100)
    (call $fd_write
      (i32.const 1)   ;; file descriptor (stdout)
      (i32.const 0)   ;; iovs pointer
      (i32.const 1)   ;; iovs count
      (i32.const 100) ;; nwritten pointer (ignored)
    )
    drop
  )
)
