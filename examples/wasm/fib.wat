;; Fibonacci Sequence in WebAssembly
;; Demonstrates recursion and loops in Wasm

(module
  ;; Recursive Fibonacci (slower but demonstrates recursion)
  (func (export "fib_recursive") (param $n i32) (result i32)
    ;; Base cases: fib(0) = 0, fib(1) = 1
    local.get $n
    i32.const 2
    i32.lt_s
    if (result i32)
      local.get $n
    else
      ;; fib(n) = fib(n-1) + fib(n-2)
      local.get $n
      i32.const 1
      i32.sub
      call $fib_recursive

      local.get $n
      i32.const 2
      i32.sub
      call $fib_recursive

      i32.add
    end
  )

  ;; Internal recursive function reference
  (func $fib_recursive (param $n i32) (result i32)
    local.get $n
    i32.const 2
    i32.lt_s
    if (result i32)
      local.get $n
    else
      local.get $n
      i32.const 1
      i32.sub
      call $fib_recursive

      local.get $n
      i32.const 2
      i32.sub
      call $fib_recursive

      i32.add
    end
  )

  ;; Iterative Fibonacci (faster, O(n) time)
  (func (export "fib_iterative") (param $n i32) (result i32)
    (local $a i32)
    (local $b i32)
    (local $temp i32)
    (local $i i32)

    ;; Base case
    local.get $n
    i32.const 2
    i32.lt_s
    if (result i32)
      local.get $n
    else
      ;; Initialize: a=0, b=1
      i32.const 0
      local.set $a
      i32.const 1
      local.set $b
      i32.const 2
      local.set $i

      ;; Loop from 2 to n
      block $done
        loop $loop
          ;; Check if i > n
          local.get $i
          local.get $n
          i32.gt_s
          br_if $done

          ;; temp = a + b
          local.get $a
          local.get $b
          i32.add
          local.set $temp

          ;; a = b
          local.get $b
          local.set $a

          ;; b = temp
          local.get $temp
          local.set $b

          ;; i++
          local.get $i
          i32.const 1
          i32.add
          local.set $i

          br $loop
        end
      end

      local.get $b
    end
  )
)
