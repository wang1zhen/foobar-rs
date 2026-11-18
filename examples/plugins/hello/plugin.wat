(module
  (memory 1)
  ;; Two 5-byte command names: "Hello" at 0, "World" at 5.
  (data (i32.const 0) "Hello")
  (data (i32.const 5) "World")

  (func $name_offset (param $index i32) (result i32)
    (if (result i32) (i32.eq (local.get $index) (i32.const 0))
      (then (i32.const 0))
      (else
        (if (result i32) (i32.eq (local.get $index) (i32.const 1))
          (then (i32.const 5))
          (else (i32.const -1))))))

  (func (export "fb_command_count") (result i32)
    (i32.const 2))

  (func (export "fb_command_name_len") (param $index i32) (result i32)
    (if (result i32) (i32.eq (local.get $index) (i32.const 0))
      (then (i32.const 5))
      (else
        (if (result i32) (i32.eq (local.get $index) (i32.const 1))
          (then (i32.const 5))
          (else (i32.const -1))))))

  (func (export "fb_command_name_byte") (param $index i32) (param $offset i32) (result i32)
    (local $base i32)
    (local.set $base (call $name_offset (local.get $index)))
    (if (result i32) (i32.lt_s (local.get $base) (i32.const 0))
      (then (i32.const -1))
      (else
        (if (result i32)
          (i32.or
            (i32.lt_s (local.get $offset) (i32.const 0))
            (i32.ge_s (local.get $offset) (i32.const 5)))
          (then (i32.const -1))
          (else
            (i32.load8_u
              (i32.add (local.get $base) (local.get $offset))))))))

  (func (export "fb_command_run") (param $index i32) (result i32)
    (local.get $index)))
