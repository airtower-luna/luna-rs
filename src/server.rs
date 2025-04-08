use crate::{set_rt_prio, ReceivedPacket, ECHO_FLAG, MIN_SIZE};
use nix::{cmsg_space, sys::{mman, socket::{self, SockaddrLike, SockaddrStorage}, time::TimeSpec}};
use std::{io::{IoSlice, IoSliceMut}, os::fd::AsRawFd};


pub fn run(bind_addr: SockaddrStorage, buf_size: usize)
		   -> Result<(), Box<dyn std::error::Error>>
{
	// prevent swapping, if possible
	if let Err(e) = mman::mlockall(
		mman::MlockAllFlags::MCL_CURRENT
			| mman::MlockAllFlags::MCL_FUTURE) {
		eprintln!("could not lock memory: {}", e);
	}

	if let Err(err) = set_rt_prio(20) {
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
	let mut buffer = vec![0u8; buf_size];
	let mut cmsgspace = cmsg_space!(TimeSpec);
	let mut iov = [IoSliceMut::new(&mut buffer)];

	println!("{}", ReceivedPacket::header());
	loop {
		let r = socket::recvmsg::<socket::SockaddrStorage>(sock.as_raw_fd(), &mut iov, Some(&mut cmsgspace), flags)?;
		let data = r.iovs().next().unwrap();

		// send echo if requested
		if r.bytes >= MIN_SIZE && 0 != (data[20] & ECHO_FLAG) {
			let iov = [IoSlice::new(data)];
			socket::sendmsg(sock.as_raw_fd(), &iov, &[], flags, r.address.as_ref())?;
		}

		if let Ok(recv) = ReceivedPacket::try_from(r) {
			println!("{recv}");
		}
	}
}
