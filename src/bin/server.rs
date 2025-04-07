use clap::Parser;
use luna_rs;
use nix::{cmsg_space, sys::{mman, socket::{self, SockaddrLike, SockaddrStorage}, time::TimeSpec}};
use std::{io::{Error, ErrorKind, IoSlice, IoSliceMut}, net::{IpAddr, SocketAddrV4, SocketAddrV6}, os::fd::AsRawFd};


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// port to listen on
    #[arg(short, long, default_value_t = 7800)]
    pub port: u16,
    /// local address to bind to for listening
    #[arg(short, long, default_value = "::")]
    pub bind: IpAddr,
    /// size of the receive buffer, larger incoming packets will be
    /// truncated
    #[arg(short, long, default_value_t = 1500)]
    pub recv_size: usize,
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let max_len = args.recv_size;
    let bind_addr: SockaddrStorage = if args.bind.is_ipv6() {
	let s = format!("[{}]:{}", args.bind, args.port);
	SockaddrStorage::from(s.parse::<SocketAddrV6>()?)
    } else {
	let s = format!("{}:{}", args.bind, args.port);
	SockaddrStorage::from(s.parse::<SocketAddrV4>()?)
    };

    // prevent swapping, if possible
    if let Err(e) = mman::mlockall(
	mman::MlockAllFlags::MCL_CURRENT
	    | mman::MlockAllFlags::MCL_FUTURE) {
	eprintln!("could not lock memory: {}", e);
    }

    if let Err(err) = luna_rs::set_rt_prio(20) {
	eprintln!("could not set realtime priority: {}", err);
    }

    let sock = socket::socket(
	bind_addr.family().unwrap(),
	socket::SockType::Datagram,
	socket::SockFlag::empty(),
	None
    )?;
    socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
    socket::bind(sock.as_raw_fd(), &bind_addr)?;

    let flags = socket::MsgFlags::empty();
    let mut buffer = vec![0u8; max_len];
    let mut cmsgspace = cmsg_space!(TimeSpec);
    let mut iov = [IoSliceMut::new(&mut buffer)];

    println!("ktime\tsource\tport\tsequence\tsize");
    loop {
	let r = socket::recvmsg::<socket::SockaddrStorage>(sock.as_raw_fd(), &mut iov, Some(&mut cmsgspace), flags)?;
	let data = r.iovs().next().unwrap();

	// send echo if requested
	if r.bytes >= luna_rs::MIN_SIZE && 0 != (data[20] & luna_rs::ECHO_FLAG) {
	    let iov = [IoSlice::new(data)];
	    socket::sendmsg(sock.as_raw_fd(), &iov, &[], flags, r.address.as_ref())?;
	}

	if let Some(socket::ControlMessageOwned::ScmTimestampns(rtime)) = r.cmsgs()?.next() {
	    let addr = r.address.as_ref().unwrap();
	    let (ip, port) = if let Some(a) = addr.as_sockaddr_in6() {
		(format!("{}", a.ip()), a.port())
	    } else { if let Some(a) = addr.as_sockaddr_in() {
		(format!("{}", a.ip()), a.port())
	    } else {
		return Err(Box::new(Error::new(ErrorKind::Unsupported, "unsupported address type")));
	    }};
	    let seq = if r.bytes >= size_of::<i32>() {
		let (s_bytes, _) = data.split_at(size_of::<i32>());
		i32::from_be_bytes(s_bytes.try_into().unwrap())
	    } else {
		// no valid sequence number
		-1
	    };
	    println!("{}.{:09}\t{}\t{}\t{}\t{}", rtime.tv_sec(), rtime.tv_nsec(), ip, port, seq, r.bytes);
	}
    }
}
