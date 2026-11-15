# 方向 2：5 Mbit/s CAN FD 与 TDC

## 先交代结果

这次选的是方向 2。固件在项目根目录用 `cargo build --locked` 完成了 `thumbv7em-none-eabihf`（STM32H723VG）的编译和链接，`host.py` 也通过了 `python3 -m py_compile host.py`。

有一点要先说清楚：**没有上板验证**。时间上没能在提交前拿到第二个 CAN FD 节点，所以下面关于收发的部分，都是根据寄存器和协议推出来的预期行为。这次提交能证明的是配置确实写进了固件、FD 帧的构造和收发路径能参与编译并生成 ELF；不能证明总线上的波形已经跑通。

## 位时序：1 Mbit/s 和 5 Mbit/s 是怎么来的

基础工程的 RCC 已经用 PLL2_Q 给 FDCAN1 供了一路 80 MHz 内核时钟：

```text
HSE 24 MHz / 12 × 240 / 6 = 80 MHz
```

CAN 的位时间按时间份额 tq 来切，一个位由同步段、TSEG1 和 TSEG2 组成：

```text
1 bit = 1 tq + TSEG1 + TSEG2
bitrate = f_kernel / (prescaler × (1 + TSEG1 + TSEG2))
sample point = (1 + TSEG1) / (1 + TSEG1 + TSEG2)
```

仲裁段沿用基础作业的配置。数据段要跑 5 Mbit/s，80 MHz 除以 5 Mbit/s 正好是 16，所以 1 位只能分到 16 个 tq：

| 阶段 | prescaler | TSEG1 | TSEG2 | SJW | 总 tq/bit | 位率 | 采样点 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 仲裁段 | 1 | 59 | 20 | 20 | 80 | 1 Mbit/s | 75% |
| 数据段 | 1 | 11 | 4 | 4 | 16 | 5 Mbit/s | 75% |

16 tq 是一个比较紧的配置。每个 tq 只有 12.5 ns，SJW 最大也就 4 tq，重同步的余量比 1 Mbit/s 小得多。这也是为什么 CAN FD 跑到 5 Mbit/s 之后，对晶振精度、总线长度和终端电阻都比 Classical CAN 敏感。

代码里这两组参数放在 `src/board.rs:30` 附近，用常量固定，并加了编译期断言。这样以后有人改 TSEG 或 prescaler，只要位率不再是 1 Mbit/s 或 5 Mbit/s，编译就会直接失败，不至于等到上板才发现配错。

## TDC：5 Mbit/s 下为什么必须开

CAN 是边发边听的协议。发送节点每发一位，都要从 RX 把总线电平读回来，确认自己发出去的位没有被别的节点改写。

问题就在于信号不是瞬间到达的。数据要从 FDCAN_TX 出发，经过收发器的 TXD、差分总线、再回到收发器的 RXD，最后进 FDCAN_RX。板载的 MCP2542FDT，TXD 到 RXD 的传播延迟典型 90 ns、最大 120 ns；而在 5 Mbit/s 下，一个数据位总共只有 200 ns。也就是说，回读边沿会晚到 7 到 10 个 tq，这个延迟已经大到不能忽略了。

如果发送端还按普通的 75% 采样点去检查回读电平，它很可能看到的是上一个位，于是把正常帧误判成位错误。TDC 做的事情，就是让发送端不要在原来的采样点检查，而是等回读边沿真正到达后，在一个延迟后的第二采样点（SSP）再检查。

我配置的参数是：

| 参数 | 值 | 单位 / 换算 |
| --- | --- | --- |
| `DBTP.TDC` | 1 | 打开 TDC |
| `TDCR.TDCO` | 12 | mtq，即 150 ns |
| `TDCR.TDCF` | 15 | mtq，即 187.5 ns |

数据段的 prescaler 是 1，所以这里 1 mtq 就等于 1 tq，也就是 12.5 ns。

TDCO 取 12，原因很直接：数据段的采样点在 `1 + DTSEG1 = 12 tq`，而 SSP 的定义是“实测环路延迟 + TDCO”。把 TDCO 设成 12，就相当于把 SSP 放回到延迟后那个位的 75% 附近。再算上收发器 7 到 10 tq 的延迟，预期 SSP 大约在 20 tq 出头。这个值远小于 6 个数据位时间（96 tq），也没有超过 127 mtq 的上限。

TDCF 取 15，是为了把滤波器打开。M_CAN 手册里写得很清楚：只有 TDCF 大于 TDCO 时，这个滤波窗口才生效；它会忽略那些会导致 SSP 过早到来的 RX 主导沿。如果测量期间总线有个毛刺，延迟可能被提前截断，SSP 就会跑得太早。TDCF 设成 15，比正常的 20 tq 出头小，正常工作时不会误伤，但能把明显偏早的毛刺挡掉。

还有两点容易混淆。第一，TDC 只在发送端的数据段起作用，仲裁段会自动关闭；对端接收时用的还是它自己的 DBTP 采样点，TDC 不会替接收端做采样。第二，上板后可以读 `PSR.TDCV`（bits 16:22，单位 mtq）看实际测到的环路延迟，如果和预期差得多，再回来微调 TDCO。

### embassy-stm32 0.6.0 其实没有写 TDC

这是这次改动里最绕的一处。

`embassy-stm32` 的 `DataBitTiming` 里有一个 `transceiver_delay_compensation` 字段，看起来传 `true` 就能打开 TDC。但翻源码会发现，`set_data_bit_timing()` 只写了 DBTP 的 DBRP、DTSEG1、DTSEG2 和 DSJW，根本没碰 TDCR；而且 `DBTP.write()` 是按整寄存器写的，会把 TDC 位清掉。也就是说，那个 bool 从头到尾没有人用。

所以只能自己补。`src/board.rs:136` 的 `enable_tdc()` 在 `into_normal_mode()` 之后重新进一次初始化模式，用 `stm32-metapac` 直接操作寄存器：

1. 置 `CCCR.INIT = 1`，有界等待，确认核真的进了初始化状态；
2. 置 `CCCR.CCE = 1`，允许修改配置；
3. 写 TDCR：TDCF = 15，TDCO = 12；
4. 用 `DBTP.modify()` 只把 TDC 位置 1，避免把刚配好的位时序覆盖掉；
5. 清 CCE、清 INIT，有界等待退出。

这里的等待都设了上限，而不是死等。目的是避免 CAN 核状态异常时卡在启动阶段，导致 USB 还没枚举就停住了。正常上电时，这几步会在很短时间内完成。

## FD 帧怎么收发

发送端用的是：

```rust
FdFrame::new(Header::new_fd(id, len, false, true), data)
```

最后一个参数是 BRS，设为 `true`；`Header::new_fd()` 同时会把 FDF 置起来。这样不管载荷是 8 字节还是 12 字节，帧头里都是 FDF = 1、BRS = 1。

这里有个坑：不要用 `FdFrame::new_standard()`。它内部走的是 Classical 的 `Header::new()`，8 字节以内的帧会变成普通 CAN 帧，超过 8 字节虽然会按 FD 发，但 BRS 是 0。看起来能发出去，实际上没有跑数据段的高位率。

接收端统一用 `Can::read_fd()`，直接从硬件帧头读 FDF 和 BRS，再按标志输出。原来的 `CAN RX` 用的是 `can.read()`，它只能转成最多 8 字节的 Classical 帧；遇到 12、32、64 字节的 FD 帧时，RX FIFO 已经被清掉，转换却失败了，结果就是帧被静默丢掉。这次把 `CAN RX` 也改成 `read_fd()` 之后，再按 `header.fdcan()` 分支，普通帧的输出格式保持不变，FD 帧也不会再丢。

CAN FD 的 DLC 是 4 位，但不是线性的。本次要求处理的 12、32、64 字节分别编码成 DLC 9、13、15：

| DLC | 0–8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 载荷字节 | 0–8 | 12 | 16 | 20 | 24 | 32 | 48 | 64 |

因为没有 9 到 11 字节这种档位，固件在解析发送长度时做了白名单，只接受 `0–8、12、16、20、24、32、48、64`。其他长度直接返回 `ERR FD LEN`，避免出现“我发了 13 字节，对端按 16 字节收”的歧义。

缓冲区方面，Embassy 0.6.0 给每个 RX/TX message RAM 元素留了 16 个 `u32`，也就是 64 字节，所以最长的 FD 帧能直接放进去。TX 队列按 3 个缓冲区配置，`fd-test` 一次连续发三帧没问题，第四帧就要等队列空出来。

## 主机命令和 USB 传输

基础的三条命令保留不变，新增了三条 FD 命令：

```text
FD TX <id> <hex>     发送 FD+BRS 标准帧，长度由 hex 字符数推断
FD RX                读一帧，返回 OK RXFD <ID> F<fd> B<brs> <len> <hex>
FD INFO              回读 NBTP / DBTP / CCCR / TDCR 的原始值
```

实际跑起来大概是这样：

```text
> FD TX 123 0102030405060708090A0B0C
OK FD QUEUED
> FD RX
OK RXFD 123 F1 B1 12 0102030405060708090A0B0C
> FD INFO
OK FD NBTP=26003A13 DBTP=00800A33 CCCR=00000300 TDCR=00000C0F
```

`OK FD QUEUED` 只能说明帧进了 3 深的发送队列，不代表总线上已经发完了。如果线上没有别的节点回 ACK，自动重传会让它一直停在待发送状态，这一点在没有对端的时候特别容易误判。

64 字节载荷转成十六进制有 128 个字符，原来的 `Reply` 只有 63 字节，肯定放不下。所以协议层的 `Line` 扩到了 256 字节，`Reply` 扩到了 192 字节。USB 这边也有类似的问题：CDC 全速单包最大 64 字节，一条回复超过这个长度就不能再当成单包发。`src/main.rs:79` 现在把回复按每包最多 63 字节切开，每一包都是短包，主机按字节流收下来就能拼回完整的一行，不需要再额外发零长度包（ZLP）。

## 代码入口和数据流

| 位置 | 作用 |
| --- | --- |
| `src/board.rs:98` `can1()` | 配置仲裁段和数据段位时序，打开 FD+BRS |
| `src/board.rs:136` `enable_tdc()` | 补写 TDCR 和 DBTP.TDC |
| `src/board.rs:168` `fd_regs()` | 给 `FD INFO` 读寄存器 |
| `src/protocol.rs:59` `execute()` | 命令分派，处理 FD TX/RX/INFO 和兼容性的 CAN RX |
| `src/protocol.rs:182` `parse_fd_frame()` | 长度白名单校验，构造 FdFrame |
| `src/main.rs:64` `control_port()` | 控制口主循环，长回复分包 |
| `host.py` | `fd-send`、`fd-recv`、`fd-info`、`fd-test` |

发送路径：

```text
主机文本命令 → parse_fd_frame() → Can::write_fd() → TX 队列 → FDCAN → 总线
```

接收路径：

```text
总线 → RX FIFO → Can::read_fd() → FdEnvelope → write_fd_frame() → USB 控制口 → 主机
```

## 哪些是复用的，哪些是自己写的

基础工程、USB、UART 和 SWD 都沿用课程模板和之前的基础作业，这部分没有重写。

`FdFrame`、`Can::write_fd/read_fd`、DLC 编解码和 message RAM 的布局来自 **embassy-stm32 0.6.0**（`can/fd/*`、`can/frame.rs`）。寄存器访问用的是 **stm32-metapac 21.0.0**，和 embassy-stm32 底层用的是同一个 PAC。

自己改的主要有三块：

- `src/board.rs` 的位时序配置，以及 `enable_tdc()` 里那段补写 TDC 的寄存器序列；
- `src/protocol.rs` 的 FD 命令、长度校验，以及 `CAN RX` 改用 `read_fd()`；
- `src/main.rs` 的长回复分包和 `host.py` 的 FD 测试命令。

参考资料主要是 ST AN5348、STM32H7 参考手册的 FDCAN 章节、Bosch M_CAN 手册，以及 MCP2542FD 的数据手册。

## 编译检查、寄存器预期和上板前的准备

编译和语法检查的命令就是作业要求的那两条：

```sh
cargo build --locked
python3 -m py_compile host.py
```

上板后可以用 `FD INFO`，或者直接用 probe-rs 读寄存器，对照下面这组值：

| 寄存器 | 预期值 | 含义 |
| --- | --- | --- |
| `NBTP` | `0x26003A13` | 仲裁段 1 Mbit/s，75% 采样点 |
| `DBTP` | `0x00800A33` | 数据段 5 Mbit/s，75% 采样点，TDC 已打开 |
| `CCCR` | `0x00000300` | FDOE（bit 8）和 BSE（bit 9）已置位 |
| `TDCR` | `0x00000C0F` | TDCO = 12，TDCF = 15 |

DBTP 的值可以按位拆开核对（H7 的字段位置是 TDC[23]、DBRP[20:16]、DTSEG1[11:8]、DTSEG2[7:4]、DSJW[3:0]，写进寄存器的都是参数减 1）：

```text
TDC    = 1        << 23 = 0x00800000
DBRP   = 1 - 1    << 16 = 0x00000000
DTSEG1 = 11 - 1   <<  8 = 0x00000A00
DTSEG2 = 4 - 1    <<  4 = 0x00000030
DSJW   = 4 - 1    <<  0 = 0x00000003
                         ----------
                         0x00800A33
```

以后有条件上板时，对端需要和这边匹配：仲裁段 1 Mbit/s、数据段 5 Mbit/s、采样点 75%，支持 FD+BRS；如果对端也发 FD 帧，它那边同样建议打开 TDC。总线两端接 120 Ω 终端，节点共地，线尽量短。

在真正跑通之前，这里只声明编译和配置逻辑通过，不声称 CAN FD 的物理收发已经验证。
