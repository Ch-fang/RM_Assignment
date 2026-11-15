# DM-MC02 USB 调试器作业测试记录

## 测试环境

- 测试时间：2026-11-13 12:00 HKT（中午）
- 记录整理：2026-11-15
- 主机：macOS 27.2
- 目标板：DM-MC02
- 外置调试探针：Horco CMSIS-DAP（`ef1a:74e5-0:7844331832`）
- Rust：1.99.1
- Cargo：1.99.1
- probe-rs：0.32.0
- Python：3.13.7
- pyserial：3.5

## 引脚表

| 信号 | MCU 引脚 | 接插件/脚号 | 说明 |
| --- | --- | --- | --- |
| SWCLK | PA0 | P11 第 1 路 PWM 信号 | 软件模拟 SWD 时钟 |
| SWDIO | PA2 | P11 第 2 路 PWM 信号 | 软件模拟 SWD 数据，双向 |
| UART TX | PA9 | P10 pin 1（USART1） | MCU → 对端 RX |
| UART RX | PA10 | P10 pin 2（USART1） | 对端 TX → MCU |

冲突检查：USB 使用 PA11/PA12，CAN1 使用 PD0/PD1，真正的调试口使用 PA13/PA14；上述 SWD 和 UART 引脚与它们不重叠。

## 实施过程中的调整与试错记录

### 1. 原计划与实际接口的差异

最初按 README 的默认建议选脚：SWD 用 P16 的 PD15/PD14（P16 pin 9/11），UART 用 UART8 的 PE0/PE1（P16 pin 15/17）。但是实际拿到板子后发现没有匹配的线去使用 P16 进行短接等操作。

因此根据板卡上实际可用的接插件重新选脚：

| 功能 | 原计划 | 实际使用 | 更改原因 |
| --- | --- | --- | --- |
| SWCLK | PD15 / P16 pin 9 | **PA0 / P11 第 1 路 PWM 信号** | P16 为 0.8 mm 间距，无法可靠接线；P11 是 2.54 mm 舵机排针，可用杜邦线 |
| SWDIO | PD14 / P16 pin 11 | **PA2 / P11 第 2 路 PWM 信号** | 同上 |
| UART TX | PE1 / P16 pin 17（UART8） | **PA9 / P10 pin 1（USART1）** | UART8 也在 P16 上；P10 有现成的 USART1 线材 |
| UART RX | PE0 / P16 pin 15（UART8） | **PA10 / P10 pin 2（USART1）** | 同上 |

替代引脚在使用前做了冲突检查：
PA0/PA2 没有被 USB、CAN1 或真正的调试口占用；PA9/PA10 也没有被 USB、CAN1 或 SWD 占用。
PA0/PA2 是 P11 上的 TIM2_CH1/TIM2_CH3，只把它们当普通 GPIO 使用，不启用 PWM。
USART1 是板上独立 UART，P10 引出了 TX/RX/GND，适合做回环测试。

### 2. 程序修改

最终 `src/wiring.rs` 相对模板的改动：

- SWCLK：`Output::new(p.PD15, Level::Low, Speed::VeryHigh)` → `Output::new(p.PA0, Level::Low, Speed::Low)`
- SWDIO：`Flex::new(p.PD14)` → `Flex::new(p.PA2)`
- UART：`BufferedUart::new(p.UART8, p.PE0, p.PE1, ...)` → `BufferedUart::new(p.USART1, p.PA10, p.PA9, ...)`
- 中断绑定：`UART8 => ...` → `USART1 => usart::BufferedInterruptHandler<peripherals::USART1>`
- 将 SWD 时钟脚的速度从 `VeryHigh` 改为 `Low`，短线连接更稳，减少过冲

注意 `BufferedUart::new` 的引脚顺序是 **RX 在前、TX 在后**，所以 USART1 传的是 `p.PA10`（RX）和 `p.PA9`（TX），不是按 TX/RX 的书写顺序。

### 3. 调试过程中遇到的问题与定位

| 阶段 | 现象 | 定位与处理 | 结果 |
| --- | --- | --- | --- |
| 探针识别 | `probe-rs list` 显示 `No debug probes were found` | DM-MC02 是目标板，不是探针；第一次烧录需要外置 CMSIS-DAP | 接入 Horco CMSIS-DAP 后识别为 `Horco CMSIS-DAP` |
| 首次烧录 | `No target responded to the debug port` | 逐项检查 P1 的 SWCLK/SWDIO/GND、板子供电和探针选择，重新接线后重试 | 烧录成功 |
| USB 枚举 | 烧录后 Mac 看不到 DM-MC02，也没有串口 | USB-C 线只能充电，不能传数据；更换为可传数据的线 | 出现 `DM-MC02 CMSIS-DAP Demo` 和两个 `/dev/cu.usbmodem*` |
| SWD 接线 | 原计划 PD15/PD14 在 P16 上，现有线材无法连接 | 改用 P11 的 PA0/PA2，重新编译、烧录、接线 | 两次 `OK DPIDR 6BA02477` |
| UART 接口 | 原计划 UART8 在 P16 上，同样无法接线 | 改用 P10 的 USART1（PA10=RX、PA9=TX） | 固件编译、烧录成功 |



## 实验一：USB 枚举与控制口

### 命令

```sh
probe-rs list
python3 -m serial.tools.list_ports
python3 host.py /dev/cu.usbmodem102 ping
```

### 实测输出

```text
The following debug probes were found:
[0]: DM-MC02 CMSIS-DAP Demo -- c0de:cafc-0: (CMSIS-DAP)
[1]: Horco CMSIS-DAP -- ef1a:74e5-0:7844331832 (CMSIS-DAP)
```

相关串口：

```text
/dev/cu.usbmodem102
/dev/cu.usbmodem104
```

控制口 PING：

```text
OK DM-MC02
```

结论：通过。`/dev/cu.usbmodem102` 为控制口，`/dev/cu.usbmodem104` 为 UART 口；两个 CDC ACM 串口和 CMSIS-DAP 演示接口均正常枚举。

## 实验二：SWD 读取 DPIDR

测试前已将外置探针从 P1 断开，并把 PA0 接到 P1 pin 3（SWCLK）、PA2 接到 P1 pin 4（SWDIO）；同板共地，不额外接 GND。

### 命令

```sh
python3 host.py /dev/cu.usbmodem102 swd
python3 host.py /dev/cu.usbmodem102 swd
python3 host.py /dev/cu.usbmodem102 ping
```

### 实测输出

```text
OK DPIDR 6BA02477
OK DPIDR 6BA02477
OK DM-MC02
```

结论：通过。两次 DPIDR 完全一致，说明 GPIO bit-bang 的 SWD 时序和接线正确；随后控制口仍能正常响应。

## 实验三：UART 回环

测试前将 USART1 的 P10 pin 1（PA9/TX）与 pin 2（PA10/RX）短接，pin 3（GND）不参与短接。

### 命令

```sh
python3 host.py /dev/cu.usbmodem104 uart-loopback
```

### 实测输出

```text
OK UART loopback
```

结论：通过。主机发送的 `HerKules UART\n` 经 USB CDC → USART1 TX → 外部短接 → USART1 RX → USB CDC 原样返回，逐字节一致。


## 未测项目

- 选做 Classical CAN 双向收发：未测，当前没有第二个能正常应答的 CAN 节点。
- 选做 CAN FD：未测。
- 选做 Linux SocketCAN：未测。

## 可复现性说明

- 串口号可能因重新插拔而变化。测试前先运行 `python3 -m serial.tools.list_ports`，再对每个 `/dev/cu.usbmodem*` 执行 `ping`，返回 `OK DM-MC02` 的是控制口，另一个是 UART 口。
- SWD 自测时，外置探针必须从 P1 断开，否则探针与 PA0/PA2 会同时驱动 SWDIO/SWCLK。
- UART 回环必须短接所选 UART 的 TX 和 RX；USART1 对应 P10 pin 1 和 pin 2，不能短接到 pin 3 的 GND。
- USART1 不在 README 默认列出的 UART8/UART7 中；本次因 P10 接口有现成线材而选择 USART1，PA9/PA10 未被 USB、CAN1、SWD 占用。