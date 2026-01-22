;; Simple Calculator in WebAssembly
;; Demonstrates exported functions for arithmetic operations

(module
  ;; Export arithmetic functions that can be called from the host

  ;; Addition: a + b
  (func (export "add") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.add
  )

  ;; Subtraction: a - b
  (func (export "sub") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.sub
  )

  ;; Multiplication: a * b
  (func (export "mul") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.mul
  )

  ;; Division: a / b (returns 0 if b is 0)
  (func (export "div") (param $a i32) (param $b i32) (result i32)
    ;; Check for division by zero
    local.get $b
    i32.eqz
    if (result i32)
      i32.const 0  ;; Return 0 on division by zero
    else
      local.get $a
      local.get $b
      i32.div_s    ;; Signed division
    end
  )

  ;; Modulo: a % b
  (func (export "mod") (param $a i32) (param $b i32) (result i32)
    local.get $b
    i32.eqz
    if (result i32)
      i32.const 0
    else
      local.get $a
      local.get $b
      i32.rem_s
    end
  )

  ;; Factorial: n!
  (func (export "factorial") (param $n i32) (result i32)
    (local $result i32)

    ;; Initialize result to 1
    i32.const 1
    local.set $result

    ;; Loop while n > 1
    block $done
      loop $loop
        ;; Check if n <= 1
        local.get $n
        i32.const 1
        i32.le_s
        br_if $done

        ;; result = result * n
        local.get $result
        local.get $n
        i32.mul
        local.set $result

        ;; n = n - 1
        local.get $n
        i32.const 1
        i32.sub
        local.set $n

        br $loop
      end
    end

    local.get $result
  )
)
