# 方向 2：5 Mbit/s CAN FD 与 TDC 实现说明

## 验收状态

- 固件通过 `cargo build --locked` 对 `thumbv7em-none-eabihf` / STM32H723VG 完成编译和链接。
- `host.py` 通过 `python3 -m py_compile host.py`。
- **硬件行为尚未上板验证**。下面的收发行为是预期结果；本次提交只证明代码可以生成固件。

## 位时序

FDCAN 内核时钟来自 PLL2_Q：

```text
HSE 24 MHz / 12 × 240 / 6 = 80 MHz
```

位率公式：

```text
bitrate = f_kernel / (prescaler × (1 + TSEG1 + TSEG2))
sample point = (1 + TSEG1) / (1 + TSEG1 + TSEG2)
```

| 段 | prescaler | TSEG1 | TSEG2 | SJW | tq/bit | 位率 | 采样点 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 仲裁段 | 1 | 59 | 20 | 20 | 80 | 1 Mbit/s | 75% |
| 数据段 | 1 | 11 | 4 | 4 | 16 | 5 Mbit/s | 75% |

数据段 `80 MHz / (1 × 16) = 5 Mbit/s`。80 MHz 下 5 Mbit/s 只能取 16 tq，1 tq = 12.5 ns；因此 SJW 最大为 4 tq，对时钟精度、总线长度和终端比 1 Mbit/s 更敏感。

代码中用常量加编译期断言固定这两组参数，见 `src/board.rs:30-61`。

## TDC

配置文件在 `src/board.rs`：

| 参数 | 值 | 单位 |
| --- | --- | --- |
| `DBTP.TDC` | 1 | — |
| `TDCR.TDCO` | 12 | mtq |
| `TDCR.TDCF` | 15 | mtq |

数据段 prescaler = 1，因此 **1 mtq = 1 tq = 12.5 ns**：

- TDCO = 12 mtq = 150 ns。
- TDCF = 15 mtq = 187.5 ns。

依据：

- ST AN5348 与 Bosch M_CAN 手册：`SSP = 实测 FDCAN_TX→FDCAN_RX 环路延迟 + TDCO`；测量分辨率为 1 mtq；`延迟 + TDCO` 必须 ≤ 127 mtq 且小于 6 个数据位时间。
- TDCF 定义 SSP 的最小值；只有 **TDCF > TDCO** 时滤波器才启用，早于该位置的 RX 主导沿会被忽略，避免测量毛刺产生过早 SSP。
- 数据段采样点在 `1 + DTSEG1 = 12 tq`。取 TDCO = 12，使 SSP 落在延迟后位的 75% 附近。
- 板载 MCP2542FDT 的 TXD→RXD 传播延迟典型 90 ns、最大 120 ns（5 Mbit/s 下约 7–10 tq）。预期 SSP ≈ 20–24 tq，满足 ≤127 mtq 和 < 96 tq 的约束。
- TDC 只在数据段生效，仲裁段自动关闭；接收端的采样点仍由 DBTP 位时序决定。
- 上板后可用 `PSR.TDCV`（bits 16:22，单位 mtq）回读实测延迟并微调 TDCO。

**自写改动**：embassy-stm32 0.6.0 的 `DataBitTiming` 虽然接受 `transceiver_delay_compensation`，但 `set_data_bit_timing()` 只写 DBTP 的 DBRP/DTSEG1/DTSEG2/DSJW，不写 TDCR，`DBTP.write()` 还会清掉 TDC 位。因此 `src/board.rs:136-166` 在 `into_normal_mode()` 之后重新进入 `INIT/CCE`，通过 `stm32-metapac` 写 `TDCR` 和 `DBTP.TDC`。

初始化顺序：

1. `CCCR.INIT = 1`，有界等待。
2. `CCCR.CCE = 1`。
3. 写 `TDCR`：TDCF=15，TDCO=12。
4. `DBTP.modify(TDC = 1)`。
5. 清 `CCE`、清 `INIT`，有界等待。

## FD 帧处理

- 发送使用 `FdFrame::new(Header::new_fd(id, len, false, true), data)`，保证 **FDF=1、BRS=1**。
- 接收使用 `Can::read_fd()`，从硬件头读取 FDF/BRS 标志。
- 合法载荷长度：0–8、12、16、20、24、32、48、64 字节；其他长度返回 `ERR FD LEN`。
- DLC 映射：12→9、32→13、64→15。

| DLC | 载荷字节 |
| --- | --- |
| 0–8 | 0–8 |
| 9 | 12 |
| 10 | 16 |
| 11 | 20 |
| 12 | 24 |
| 13 | 32 |
| 14 | 48 |
| 15 | 64 |

缓冲区：Embassy 0.6.0 的 FDCAN message RAM 每个 RX/TX 元素有 16 个 u32 = **64 字节**数据空间；TX 队列深度为 3，所以 `fd-test` 一次最多发 3 帧。

### 主机命令

```text
FD TX <id> <hex>     # 发送 FD+BRS 标准帧；长度由 hex 推断
FD RX                # 读取一帧，返回 OK RXFD <ID> F<fd> B<brs> <len> <hex>
FD INFO              # 返回 NBTP/DBTP/CCCR/TDCR 原始寄存器值
CAN TX / CAN RX      # 保留；CAN RX 改为 read_fd 后按 FDF 分支
```

示例：

```text
> FD TX 123 0102030405060708090A0B0C
OK FD QUEUED
> FD RX
OK RXFD 123 F1 B1 12 0102030405060708090A0B0C
> FD INFO
OK FD NBTP=26003A13 DBTP=00800A33 CCCR=00000300 TDCR=00000C0F
```

`OK FD QUEUED` 只表示进入 3 深发送队列，不代表总线上发送完成。没有 ACK 节点时，自动重传会让帧一直处于待发送状态。

### USB 帧边界

CDC 全速单包最大 64 字节。控制口把一条文本回复按 **≤63 字节**分包写出（`src/main.rs:79`），每包都是短包，主机按字节流重组，不需要 ZLP。`Line` 缓冲 256 字节、`Reply` 192 字节，足以容纳 64 字节载荷的十六进制文本。

## 数据路径与入口

| 位置 | 作用 |
| --- | --- |
| `src/board.rs:98-133` `can1()` | 设置 1 Mbit/s + 5 Mbit/s 位时序、FD+BRS |
| `src/board.rs:136-166` `enable_tdc()` | 写 TDCR 与 DBTP.TDC |
| `src/board.rs:168-176` `fd_regs()` | 供 `FD INFO` 读取寄存器 |
| `src/protocol.rs:40-134` `execute()` | 命令分派、FD TX/RX/INFO、CAN RX |
| `src/protocol.rs:182-203` `parse_fd_frame()` | 长度白名单与 FdFrame 构造 |
| `src/main.rs:64-100` `control_port()` | 主循环与回复分包 |
| `host.py` | `fd-send`、`fd-recv`、`fd-info`、`fd-test` |

发送路径：主机文本命令 → `parse_fd_frame()` → `Can::write_fd()` → TX 队列 → FDCAN → 总线。

接收路径：总线 → RX FIFO → `Can::read_fd()` → `FdEnvelope` → `write_fd_frame()` → USB 控制口 → 主机。

## 复用来源与自己改动

- 基础工程与 USB/UART/SWD 代码来自课程模板和本仓库基础作业。
- `FdFrame`、`Can::write_fd/read_fd`、DLC 编解码、message RAM 布局来自 **embassy-stm32 0.6.0**（`can/fd/*`、`can/frame.rs`）。
- 寄存器访问使用 **stm32-metapac 21.0.0**（`stm32h723vg` feature），这是 embassy-stm32 底层使用的同一 PAC。
- TDC 初始化序列是本项目自写改动，原因是 embassy 0.6.0 不写 TDC；对应代码在 `src/board.rs:136-166`。
- 参考文档：ST AN5348、ST RM0433（FDCAN 寄存器）、Bosch M_CAN User's Manual、Microchip MCP2542FD 数据手册。

## 编译与测试

```sh
cargo build --locked
python3 -m py_compile host.py
```

预期寄存器值（`FD INFO` 或 probe-rs 回读）：

| 寄存器 | 预期值 |
| --- | --- |
| NBTP | `0x26003A13` |
| DBTP | `0x00800A33` |
| CCCR | `0x00000300`（FDOE+BSE） |
| TDCR | `0x00000C0F` |

上板验证需要第二个 CAN FD 节点或 USB-CAN-FD 分析仪，双方配置：

- 仲裁段 1 Mbit/s、数据段 5 Mbit/s、采样点 75%。
- 支持 FD+BRS；对端发送时也建议开启 TDC。
- 总线两端 120 Ω 终端、共地、短线。

在具备条件前，本仓库不声称 FD 收发已通过硬件验证。
