use nix::{cmsg_space, libc::timespec, sys::{socket, time::TimeSpec}};
use std::{io::{IoSlice, IoSliceMut}, os::fd::AsRawFd, str::FromStr};

const ECHO_FLAG: u8 = 1;
const MIN_SIZE: usize = size_of::<u32>() + size_of::<timespec>() + size_of::<u8>();


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let max_len = 1500;
    let sock = socket::socket(
	socket::AddressFamily::Inet6,
	socket::SockType::Datagram,
	socket::SockFlag::empty(),
	None
    )?;
    socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
    let any = socket::SockaddrIn6::from_str("[::]:7800")?;
    socket::bind(sock.as_raw_fd(), &any)?;

    let flags = socket::MsgFlags::empty();
    let mut buffer = vec![0u8; max_len];
    let mut cmsgspace = cmsg_space!(TimeSpec);
    let mut iov = [IoSliceMut::new(&mut buffer)];

    println!("ktime\tsource\tport\tsequence\tsize");
    loop {
	let r = socket::recvmsg::<socket::SockaddrIn6>(sock.as_raw_fd(), &mut iov, Some(&mut cmsgspace), flags)?;
	let data = r.iovs().next().unwrap();

	// send echo if requested
	if r.bytes >= MIN_SIZE && 0 != (data[20] & ECHO_FLAG) {
	    let iov = [IoSlice::new(data)];
	    socket::sendmsg(sock.as_raw_fd(), &iov, &[], flags, r.address.as_ref())?;
	}

	if let Some(socket::ControlMessageOwned::ScmTimestampns(rtime)) = r.cmsgs()?.next() {
	    let addr = r.address.as_ref().unwrap();
	    let seq = if r.bytes >= size_of::<i32>() {
		let (s_bytes, _) = data.split_at(size_of::<i32>());
		i32::from_be_bytes(s_bytes.try_into().unwrap())
	    } else {
		// no valid sequence number
		-1
	    };
	    println!("{}.{:09}\t{}\t{}\t{}\t{}", rtime.tv_sec(), rtime.tv_nsec(), addr.ip(), addr.port(), seq, r.bytes);
	}
    }
}
