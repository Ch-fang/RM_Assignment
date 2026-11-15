#!/usr/bin/env python3
"""Exercise the two serial interfaces of the classroom bridge."""

import argparse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("port", help="USB serial device name (for example /dev/ttyACM0)")
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("ping", "swd", "can-recv", "fd-recv", "fd-info", "fd-test", "uart-loopback"):
        commands.add_parser(command)
    send = commands.add_parser("can-send")
    send.add_argument("id", help="11-bit hex ID, without 0x")
    send.add_argument("data", help="0–8 bytes in hex, or '-' for no data")
    fd_send = commands.add_parser("fd-send")
    fd_send.add_argument("id", help="11-bit hex ID, without 0x")
    fd_send.add_argument(
        "data",
        help="0-8, 12, 16, 20, 24, 32, 48 or 64 bytes in hex, or '-'",
    )
    args = parser.parse_args()

    try:
        import serial
    except ImportError:
        parser.exit(2, "Install pyserial 3.5 in a project environment first.\n")

    try:
        with serial.Serial(args.port, 115200, timeout=2, write_timeout=2) as port:
            port.reset_input_buffer()
            if args.command == "uart-loopback":
                expected = b"HerKules UART\n"
                port.write(expected)
                got = port.read(len(expected))
                if got != expected:
                    parser.exit(1, f"Expected {expected!r}, received {got!r}\n")
                print("OK UART loopback")
                return
            if args.command == "fd-test":
                # Three frames fit the FDCAN TX queue exactly.
                for data in (
                    "0102030405060708090A0B0C",
                    "0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20",
                    "0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20"
                    "2122232425262728292A2B2C2D2E2F303132333435363738393A3B3C3D3E3F40",
                ):
                    port.write(f"FD TX 123 {data}\n".encode("ascii"))
                    reply = port.readline(320).decode("ascii", errors="replace").strip()
                    if not reply:
                        parser.exit(1, "No response. Check that this is the control interface.\n")
                    print(reply)
                    if not reply.startswith("OK "):
                        parser.exit(1)
                return
            if args.command == "can-send":
                command = f"CAN TX {args.id} {args.data}"
            elif args.command == "fd-send":
                command = f"FD TX {args.id} {args.data}"
            else:
                command = {
                    "ping": "PING",
                    "swd": "SWD ID",
                    "can-recv": "CAN RX",
                    "fd-recv": "FD RX",
                    "fd-info": "FD INFO",
                }[args.command]
            port.write((command + "\n").encode("ascii"))
            reply = port.readline(320).decode("ascii", errors="replace").strip()
            if not reply:
                parser.exit(1, "No response. Check that this is the control interface.\n")
            print(reply)
            if not (reply.startswith("OK ") or reply == "EMPTY"):
                parser.exit(1)
    except serial.SerialException as err:
        parser.exit(1, f"Serial error: {err}\n")


if __name__ == "__main__":
    main()
