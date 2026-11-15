# DM-MC02 USB 调试器作业

这是 HerKules L4 大作业的最终完成版。作业目标是把 DM-MC02（STM32H723VG）做成一个 USB 调试探针和串口桥：电脑通过 USB-C 看到两个串口和一个 CMSIS-DAP 演示接口，控制口可以执行 `PING`、`SWD ID`、`CAN TX/RX` 和 CAN FD 命令，UART 口把数据转发到板上的串口。

选做CAN作业的扩展方向我选择了 **方向 2：5 Mbit/s CAN FD 与 TDC**。详细计算、寄存器值和实现说明见 [`CAN_FD_TDC.md`](CAN_FD_TDC.md)。

## 引脚表

| 信号 | MCU 引脚 | 接插件/脚号 |
| --- | --- | --- |
| SWCLK | PA0 | P11 第 1 路 PWM 信号 |
| SWDIO | PA2 | P11 第 2 路 PWM 信号 |
| UART TX | PA9 | P10 pin 1（USART1） |
| UART RX | PA10 | P10 pin 2（USART1） |

模板原本建议 SWD 用 P16 的 PD15/PD14、UART 用 UART8 的 PE0/PE1，但线下去做测试的时候发现 P16 是 2×12、0.8 mm 间距，当时没有转接板，接不上，所以测试中紧急改成了 P11 和 P10。最后的程序在 `src/wiring.rs` 。

## 完成情况

- USB 枚举和控制口：通过，`PING` 返回 `OK DM-MC02`。
- SWD 自测：通过，两次 `SWD ID` 都返回 `OK DPIDR 6BA02477`。
- UART 回环：通过，返回 `OK UART loopback`。
- 方向 2 CAN FD + TDC：代码完成，`cargo build --locked` 编译链接通过。时间原因无法去线下上板验证。
- Classical CAN 命令保留；`CAN RX` 现在也能读出 FD 帧（按 FDF 标志分支）。

## 编译和烧录

```sh
cargo build --locked
cargo run --locked
```

第一次烧录要用外置探针接 P1。

## 测试

烧录后先找控制口：

```sh
probe-rs list
python3 -m serial.tools.list_ports
python3 host.py <CONTROL_PORT> ping
```

SWD 自测前把外置探针从 P1 断开，PA0 接 P1 pin 3，PA2 接 P1 pin 4：

```sh
python3 host.py <CONTROL_PORT> swd
python3 host.py <CONTROL_PORT> swd
python3 host.py <CONTROL_PORT> ping
```

UART 回环前短接 P10 pin 1（PA9/TX）和 pin 2（PA10/RX）：

```sh
python3 host.py <UART_PORT> uart-loopback
```

CAN FD 相关命令（需要支持 FD 的对端才能实际收发）：

```sh
python3 host.py <CONTROL_PORT> fd-info
python3 host.py <CONTROL_PORT> fd-send 123 0102030405060708090A0B0C
python3 host.py <CONTROL_PORT> fd-recv
python3 host.py <CONTROL_PORT> fd-test
```

`fd-test` 依次发送 12、32、64 字节固定载荷，最多 3 帧（对应 FDCAN 的 3 个 TX 缓冲区）。

详细的命令、输出和试错过程见 [`TEST_RECORD.md`](TEST_RECORD.md)；方向 2 的寄存器值、计算和未验证声明见 [`CAN_FD_TDC.md`](CAN_FD_TDC.md)。


