use nix::{cmsg_space, libc::timespec, sys::{mman, socket, time::TimeSpec}};
use std::{io::{Error, IoSlice, IoSliceMut}, os::fd::AsRawFd, str::FromStr};

const ECHO_FLAG: u8 = 1;
const MIN_SIZE: usize = size_of::<u32>() + size_of::<timespec>() + size_of::<u8>();


/// Enable realtime scheduling for the current process. The offset is
/// the priority relative to the minimum realtime priority. Requires
/// CAP_SYS_NICE capability.
fn set_rt_prio(offset: i32) -> Result<(), Error> {
    let min_rt_prio = unsafe {
	libc::sched_get_priority_min(libc::SCHED_RR)
    };
    let max_rt_prio = unsafe {
	libc::sched_get_priority_max(libc::SCHED_RR)
    };
    let sparam = libc::sched_param{
	sched_priority: max_rt_prio.min(min_rt_prio + offset)
    };
    let ret = unsafe {
	libc::sched_setscheduler(0, libc::SCHED_RR, &sparam)
    };
    if ret < 0 {
	Err(Error::last_os_error())
    } else {
	Ok(())
    }
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let max_len = 1500;

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
