# 选做作业——方向2：5 Mbit/s CAN FD 与 TDC 实现说明

## 完成情况

- 固件通过 `cargo build --locked` 对 `thumbv7em-none-eabihf` / STM32H723VG 完成编译和链接。
- `host.py` 通过 `python3 -m py_compile host.py`。
- **尚未上板验证**。由于时间原因无法在周六上午前完成所有内容，因此未能进行实际的测试。下面的收发行为是预期结果；本次提交只证明代码可以生成固件。

## 核心内容：

### 1. 位时序计算（80 MHz 内核时钟）

基础工程的 RCC 已经通过 `PLL2_Q` 给 `FDCAN1` 提供了一路 **80 MHz** 的内核时钟：

```text
HSE 24 MHz / 12 × 240 / 6 = 80 MHz
```

两段位时序按标准的位率与采样点公式计算：

```text
bitrate = f_kernel / (prescaler × (1 + TSEG1 + TSEG2))
sample point = (1 + TSEG1) / (1 + TSEG1 + TSEG2)
```
为了兼顾仲裁阶段的稳定性与数据阶段的传输速度，两段分别配置如下：

| 段 | prescaler | TSEG1 | TSEG2 | SJW | tq/bit | 位率 | 采样点 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 仲裁段 | 1 | 59 | 20 | 20 | 80 | 1 Mbit/s | 75% |
| 数据段 | 1 | 11 | 4 | 4 | 16 | 5 Mbit/s | 75% |

数据段 `80 MHz / (1 × 16) = 5 Mbit/s`。80 MHz 下 5 Mbit/s 只能取 16 tq，1 tq = 12.5 ns；因此 SJW 最大为 4 tq，对时钟精度、总线长度和终端比 1 Mbit/s 更敏感。

代码中在 `src/board.rs:30-61` 用常量固定了这两组参数，并添加了编译期 `const` 断言，防止后面改参数时算错位率。

### 2. TDC原理与补写

5 Mbit/s 下一个数据位只有 `200 ns`，而板载收发器 MCP2542FDT 的 `TXD → RXD` 传播延迟典型值就有 `90 ns`、最大 `120 ns`（大约占了 7～10 个 tq）。如果发送端还按常规采样点去回读总线电平，很可能会读到上一个旧位而误报位错误，所以数据段必须开启 TDC。

在 `src/board.rs` 中配置的参数如下（数据段 `prescaler = 1`，因此最小时间单位 **1 mtq = 1 tq = 12.5 ns**）：

| 参数 | 值 | 单位 |
| --- | --- | --- |
| `DBTP.TDC` | 1 | — |
| `TDCR.TDCO` | 12 | mtq |
| `TDCR.TDCF` | 15 | mtq |

这几个值的设定主要参考了 ST AN5348 和 Bosch M_CAN 手
册：

- **第二采样点（SSP）位置**：硬件会按 `1 mtq` 分辨率自动测量实际的 `FDCAN_TX → FDCAN_RX` 环路延迟，再加上偏移量得到 `SSP = 实测环路延迟 + TDCO`。因为数据段采样点在 `1 + DTSEG1 = 12 tq`，取 `TDCO = 12 mtq` 刚好能让 SSP 落在延迟后回读位的 75% 附近。结合收发器 7～10 tq 的延迟，预期 SSP 大约在 `20–24 tq`，满足 `≤ 127 mtq` 且小于 6 个数据位时间（`96 tq`）的约束。

- **滤波窗口的作用**：只有当 `TDCF > TDCO`（这里 `15 > 12`）时硬件滤波器才会启用。它会忽略早于 `TDCF` 位置的 RX 主导沿，避免总线毛刺导致过早触发采样。

- **作用范围**：TDC 只在发送端的数据阶段生效，仲裁段会自动关闭；对端接收节点的采样点依然由它自己的 `DBTP` 位时序决定。后续上板时，可以通过读取 `PSR.TDCV`（bits 16:22，单位 mtq）拿到实测环路延迟，再按需微调 `TDCO`。

**自写改动**：embassy-stm32 0.6.0 的 `DataBitTiming` 虽然接受 `transceiver_delay_compensation`，但 `set_data_bit_timing()` 只写 DBTP 的 DBRP/DTSEG1/DTSEG2/DSJW，不写 TDCR，`DBTP.write()` 还会清掉 TDC 位。因此 `src/board.rs:136` 的 `enable_tdc()` 在 `into_normal_mode()` 之后重新进入 `INIT/CCE`，通过 `stm32-metapac` 写 `TDCR` 和 `DBTP.TDC`。

初始化顺序：

1. 置位 `CCCR.INIT = 1`，并加有界等待确认进入初始化状态；
2. 置位 `CCCR.CCE = 1` 允许修改配置；
3. 写入 `TDCR`（`TDCF = 15`，`TDCO = 12`）；
4. 使用 `DBTP.modify(TDC = 1)` 打开 TDC（避免覆盖前面配好的位时序）；
5. 清除 `CCE` 和 `INIT`，有界等待退出初始化模式。

### 3. FD 帧处理与通信链路改动

#### 帧构造、DLC 映射与缓冲容量
- **收发接口**：发送端使用 `FdFrame::new(Header::new_fd(id, len, false, true), data)` 构造报文，确保帧头中的 **FDF=1、BRS=1**；接收端统一使用 `Can::read_fd()`，直接从硬件帧头读取 FDF 和 BRS 标志。

- **长度白名单与 DLC 映射**：CAN FD 的 4 位 `DLC` 采用非线性编码（本次重点处理的 12、32、64 字节分别对应 `DLC = 9、13、15`）。固件在解析发送长度时做了白名单校验，只放行 `0–8、12、16、20、24、32、48、64` 字节，其他长度直接返回 `ERR FD LEN`，避免非标长度被底层强行补齐造成歧义：

| DLC | 0–8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **载荷字节** | 0–8 | **12** | 16 | 20 | 24 | **32** | 48 | **64** |

- **片上缓冲队列**：Embassy 0.6.0 默认给 FDCAN Message RAM 的每个 RX/TX 元素分配了 16 个 `u32`（即 **64 字节**）数据空间，硬件上可以直接容纳最长 FD 帧。由于硬件 TX 队列深度为 3，上位机 `fd-test` 一次最多连续发送 3 帧测试报文。

#### 主机命令设计
在保留原有 `CAN TX` 命令、并将 `CAN RX` 改为 `read_fd()` 按 `FDF` 标志分支输出（防止旧版 `read()` 遇到长帧时静默丢帧）的同时，新增了三条 FD 命令：

```text
FD TX <id> <hex>     # 发送 FD+BRS 标准帧，数据长度由 hex 字符推断
FD RX                # 读取一帧，返回 OK RXFD <ID> F<fd> B<brs> <len> <hex>
FD INFO              # 回读 NBTP / DBTP / CCCR / TDCR 四个寄存器的原始值
```

预期交互示例：
```text
> FD TX 123 0102030405060708090A0B0C
OK FD QUEUED
> FD RX
OK RXFD 123 F1 B1 12 0102030405060708090A0B0C
> FD INFO
OK FD NBTP=26003A13 DBTP=00800A33 CCCR=00000300 TDCR=00000C0F
```
*(注：收到 `OK FD QUEUED` 只代表报文已进入 3 深的硬件发送队列，不代表总线上发送完成；若总线上没有 ACK 节点，自动重传会让帧一直处于待发送状态。)*

#### USB 帧边界与分包
CDC 全速模式单包最大 64 字节，而 64 字节载荷的十六进制文本加上命令头超过了原来的 96 字节限制。因此我将协议层接收缓冲 `Line` 扩大到了 **256 字节**、回复缓冲 `Reply` 扩大到了 **192 字节**。同时，在 `src/main.rs:79` 的控制口发送逻辑中，把一条文本回复按 **≤ 63 字节**分包写出，让每一包都是短包，主机端按字节流重组即可完整收到长回复，无需额外处理零长度包（ZLP）。

---

## 数据路径与入口

| 位置 | 作用 |
| --- | --- |
| `src/board.rs:98` `can1()` | 设置 1 Mbit/s + 5 Mbit/s 位时序、FD+BRS |
| `src/board.rs:136` `enable_tdc()` | 写 TDCR 与 DBTP.TDC |
| `src/board.rs:168` `fd_regs()` | 供 `FD INFO` 读取寄存器 |
| `src/protocol.rs:59` `execute()` | 命令分派、FD TX/RX/INFO、CAN RX |
| `src/protocol.rs:182` `parse_fd_frame()` | 长度白名单与 FdFrame 构造 |
| `src/main.rs:64` `control_port()` | 主循环与回复分包 |
| `host.py` | `fd-send`、`fd-recv`、`fd-info`、`fd-test` |

发送路径：主机文本命令 → `parse_fd_frame()` → `Can::write_fd()` → TX 队列 → FDCAN → 总线。

接收路径：总线 → RX FIFO → `Can::read_fd()` → `FdEnvelope` → `write_fd_frame()` → USB 控制口 → 主机。

## 复用来源与改动说明

- 基础工程与 USB/UART/SWD 代码来自课程模板和本仓库基础作业。
- `FdFrame`、`Can::write_fd/read_fd`、DLC 编解码、message RAM 布局来自 **embassy-stm32 0.6.0**（`can/fd/*`、`can/frame.rs`）。
- 寄存器访问使用 **stm32-metapac 21.0.0**（`stm32h723vg` feature），这是 embassy-stm32 底层使用的同一 PAC。
- TDC 初始化序列是本项目自写改动，原因是 embassy 0.6.0 不写 TDC；对应代码在 `src/board.rs:136` 的 `enable_tdc()`。
- 参考文档：ST AN5348、ST RM0433（FDCAN 寄存器）、Bosch M_CAN User's Manual、Microchip MCP2542FD 数据手册。

## 编译与测试

```sh
cargo build --locked
python3 -m py_compile host.py
```

### 工具链

- Rust：`rustc 1.99.1 (58cf8f9c3 2026-10-13)`
- Cargo：`cargo 1.99.1 (5f94df478 2026-08-27)`
- 当前工具链：`1.99.1-aarch64-apple-darwin`（由 `rust-toolchain.toml` 固定）
- 目标：`thumbv7em-none-eabihf`（STM32H723VG）
- 组件：`rustfmt`、`clippy`

`python3 -m py_compile host.py` 成功时没有输出，无输出即通过。

### 完整编译成功日志

为确认不是增量缓存，下面是在 `cargo clean` 之后重新执行 `cargo build --locked` 的完整输出；提交命令仍然是 `cargo build --locked`。

```text
   Compiling proc-macro2 v1.0.107
   Compiling quote v1.0.47
   Compiling unicode-ident v1.0.27
   Compiling semver-parser v0.7.0
   Compiling nb v1.1.0
   Compiling embedded-hal v1.0.0
   Compiling cortex-m-rt v0.7.7
   Compiling cortex-m-macros v0.7.8
   Compiling critical-section v1.2.0
   Compiling stable_deref_trait v1.2.1
   Compiling nb v0.1.3
   Compiling void v1.0.2
   Compiling cortex-m v0.7.9
   Compiling heapless v0.9.3
   Compiling byteorder v1.5.0
   Compiling embassy-time-driver v0.2.2
   Compiling semver v0.9.0
   Compiling embedded-hal v0.2.7
   Compiling rustc_version v0.2.3
   Compiling hash32 v0.3.1
   Compiling futures-core v0.3.34
   Compiling bare-metal v0.2.5
   Compiling embedded-io v0.7.1
   Compiling litrs v1.0.1
   Compiling embassy-sync v0.8.0
   Compiling embedded-io-async v0.7.0
   Compiling cfg-if v1.0.5
   Compiling embedded-hal-async v1.0.0
   Compiling autocfg v1.5.1
   Compiling document-features v0.2.12
   Compiling num-traits v0.2.19
   Compiling embedded-io-async v0.6.1
   Compiling vcell v0.1.3
   Compiling futures-sink v0.3.34
   Compiling memchr v2.8.3
   Compiling volatile-register v0.2.2
   Compiling bitfield v0.13.2
   Compiling syn v2.0.119
   Compiling syn v3.0.9
   Compiling aho-corasick v1.1.5
   Compiling portable-atomic v1.15.0
   Compiling ident_case v1.0.1
   Compiling embedded-io v0.6.1
   Compiling strsim v0.11.1
   Compiling stm32-metapac v21.0.0
   Compiling fnv v1.0.7
   Compiling regex-syntax v0.8.11
   Compiling embassy-time v0.5.1
   Compiling embassy-futures v0.1.2
   Compiling syn v1.0.109
   Compiling embassy-time-queue-utils v0.3.2
   Compiling embassy-hal-internal v0.4.0
   Compiling embassy-usb-driver v0.2.2
   Compiling as-slice v0.2.1
   Compiling embassy-hal-internal v0.5.0
   Compiling embassy-executor-timer-queue v0.1.0
   Compiling embassy-net-driver v0.2.0
   Compiling embedded-storage v0.3.2
   Compiling aligned v0.4.3
   Compiling embedded-storage-async v0.4.2
   Compiling futures-task v0.3.34
   Compiling bitflags v2.13.2
   Compiling embassy-usb v0.6.0
   Compiling pin-project-lite v0.2.17
   Compiling regex-automata v0.4.18
   Compiling embassy-executor v0.10.0
   Compiling futures-util v0.3.34
   Compiling embassy-embedded-hal v0.6.0
   Compiling block-device-driver v0.2.0
   Compiling embassy-net-driver-channel v0.4.0
   Compiling embassy-usb-synopsys-otg v0.3.3
   Compiling embedded-hal-nb v1.0.0
   Compiling stm32-fmc v0.4.0
   Compiling embedded-can v0.4.1
   Compiling static_assertions v1.1.0
   Compiling darling_core v0.20.11
   Compiling cordyceps v0.3.5
   Compiling rand_core v0.9.5
   Compiling rand_core v0.6.4
   Compiling sdio-host v0.9.0
   Compiling bit_field v0.10.3
   Compiling static_cell v2.1.1
   Compiling panic-halt v1.0.0
   Compiling trait-set v0.3.0
   Compiling regex v1.13.1
   Compiling cortex-m-rt-macros v0.7.7
   Compiling darling_macro v0.20.11
   Compiling darling v0.20.11
   Compiling embassy-executor-macros v0.8.0
   Compiling embassy-stm32 v0.6.0
   Compiling dm-mc02-cmsis-dap v0.1.0 (/Users/a347997/Desktop/HKU/y2/sem1/robomaster/ass 3/project/RM_Assignment)
    Finished `dev` profile [optimized + debuginfo] target(s) in 11.10s
```

## 预期寄存器回读值（`FD INFO` 或 `probe-rs` 读取）

| 寄存器 | 预期值 | 说明 |
| :--- | :---: | :--- |
| `NBTP` | `0x26003A13` | 仲裁段 1 Mbit/s，75% 采样点 |
| `DBTP` | `0x00800A33` | 数据段 5 Mbit/s，75% 采样点，已置位 `TDC` |
| `CCCR` | `0x00000300` | 已使能 `FDOE`（bit 8）与 `BSE`（bit 9） |
| `TDCR` | `0x00000C0F` | `TDCO = 12 (0x0C)`，`TDCF = 15 (0x0F)` |

其中 `DBTP` 的寄存器位域计算如下（H7 字段位置：`TDC[23]`、`DBRP[20:16]`、`DTSEG1[11:8]`、`DTSEG2[7:4]`、`DSJW[3:0]`，寄存器写入值均为实际参数减 1）：

```text
TDC    = 1        << 23 = 0x00800000
DBRP   = 1 - 1    << 16 = 0x00000000
DTSEG1 = 11 - 1   <<  8 = 0x00000A00
DTSEG2 = 4 - 1    <<  4 = 0x00000030
DSJW   = 4 - 1    <<  0 = 0x00000003
                         ----------
                         0x00800A33
```

### 后续上板验证条件

等后续具备第二个 CAN FD 节点或 USB-CAN-FD 分析仪时，按以下条件匹配后进行实际测试：
- 双方配置一致：仲裁段 1 Mbit/s、数据段 5 Mbit/s、采样点 75%；
- 支持 CAN FD + BRS，且对端发送时同样建议开启 TDC；
- 总线两端接好 `120 Ω` 终端电阻，节点共地并使用短线连接。
