# DM-MC02 USB 调试器作业

这是 HerKules L4 的最终作业。目标是把 DM-MC02（STM32H723VG）做成一个 USB 调试探针和串口桥：电脑通过 USB-C 看到两个串口和一个 CMSIS-DAP 演示接口，控制口可以执行 `PING`、`SWD ID`、`CAN TX/RX`，UART 口把数据转发到板上的串口。

## 实际引脚

| 信号 | MCU 引脚 | 接插件/脚号 |
| --- | --- | --- |
| SWCLK | PA0 | P11 第 1 路 PWM 信号 |
| SWDIO | PA2 | P11 第 2 路 PWM 信号 |
| UART TX | PA9 | P10 pin 1（USART1） |
| UART RX | PA10 | P10 pin 2（USART1） |

模板原本建议 SWD 用 P16 的 PD15/PD14、UART 用 UART8 的 PE0/PE1，但 P16 是 2×12、0.8 mm 间距，手头没有转接板，接不上，所以改成了 P11 和 P10。改动都在 `src/wiring.rs` 里。

## 完成情况

- USB 枚举和控制口：通过，`PING` 返回 `OK DM-MC02`。
- SWD 自测：通过，两次 `SWD ID` 都返回 `OK DPIDR 6BA02477`。
- UART 回环：通过，返回 `OK UART loopback`。
- CAN 选做：未做，没有第二个能应答的 CAN 节点。

## 编译和烧录

```sh
cargo build --locked
cargo run --locked
```

第一次烧录要用外置探针接 P1；DM-MC02 自己的演示接口不能烧录。

## 怎么测试

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

详细的命令、输出和试错过程见 [`TEST_RECORD.md`](TEST_RECORD.md)。

USART1 不是模板默认列出的 UART8/UART7；如果课程要求必须用那两个接口，需要改接对应引脚。
