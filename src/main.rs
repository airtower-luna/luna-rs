use nix::sys::time::TimeSpec;
use nix::time::{ClockId, ClockNanosleepFlags, clock_gettime, clock_nanosleep};

static CLOCK: ClockId = ClockId::CLOCK_MONOTONIC;


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut t = clock_gettime(CLOCK)?;
    println!("monotonic time (start): {}", t);

    let end = t + TimeSpec::new(10, 0);
    let step = TimeSpec::new(1, 0);

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

	let current = clock_gettime(CLOCK)?;
	println!("monotonic time: {}", current);
	if current > end {
	    break;
	}
    }
    Result::Ok(())
}
