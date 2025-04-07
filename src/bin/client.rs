use luna_rs;

use std::io::IoSlice;
use std::os::fd::AsRawFd;
use std::str::FromStr;

use nix::sys::{mman, socket, time::TimeSpec};
use nix::time::{ClockId, ClockNanosleepFlags, clock_gettime, clock_nanosleep};

static CLOCK: ClockId = ClockId::CLOCK_MONOTONIC;


fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: should be configurable
    let echo = true;
    let server = socket::SockaddrIn6::from_str("[::1]:7800")?;

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
	socket::AddressFamily::Inet6,
	socket::SockType::Datagram,
	socket::SockFlag::empty(),
	None
    )?;
    socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
    socket::connect(sock.as_raw_fd(), &server)?;

    let flags = socket::MsgFlags::empty();
    let mut buffer = vec![0u8; luna_rs::MIN_SIZE];
    if echo {
	buffer[20] = luna_rs::ECHO_FLAG;
    }

    let mut t = clock_gettime(CLOCK)?;
    println!("monotonic time (start): {}", t);

    let end = t + TimeSpec::new(10, 0);
    let step = TimeSpec::new(1, 0);
    let mut seq: u32 = 0;

    loop {
	t = t + step;
	loop {
	    match clock_nanosleep(CLOCK, ClockNanosleepFlags::TIMER_ABSTIME, &t) {
		Ok(_) => break,
		// restart sleep if it was interrupted
		Err(nix::Error::EINTR) => (),
		Err(e) => return Result::Err(Box::new(e))
	    }
	}

	// write current time to packet
	let current = clock_gettime(CLOCK)?;
	buffer.splice(4..12, current.tv_sec().to_be_bytes());
	buffer.splice(12..20, current.tv_nsec().to_be_bytes());

	let iov = [IoSlice::new(&buffer)];
	socket::sendmsg(sock.as_raw_fd(), &iov, &[], flags, Some(&server))?;
	#[cfg(debug_assertions)]
	println!("sent {}: {:?}", seq, buffer);
	if current > end {
	    break;
	}

	// prepare next packet
	seq += 1;
	buffer.splice(0..4, seq.to_be_bytes());
    }
    Result::Ok(())
}
