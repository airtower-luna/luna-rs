#!/bin/bash
# Run tests and report coverage. Loosely based on:
# https://doc.rust-lang.org/rustc/instrument-coverage.html#test-coverage
set -e
profdata=merged.profdata
outdir=coverage

run() {
	# build and run tests with coverage instrumentation
	RUSTFLAGS="-C instrument-coverage" cargo test --tests

	# merge raw result files
	llvm-profdata merge -sparse -o "${profdata}" default_*.profraw

	# test executables to check for coverage
	test_objects=$(
		for file in $(
			RUSTFLAGS="-C instrument-coverage" \
					cargo test --tests --no-run --message-format=json \
				| jq -r "select(.profile.test == true) | .filenames[]" \
				| grep -v dSYM -
		); do
			printf "%s %s " --object $file
		done
	)

	# paths to ignore for coverage (standard library and external crates)
	ignore_paths=$(
		for file in '/.cargo/registry' '/usr/src'; do
			printf "%s=%s " '--ignore-filename-regex' "$file"
		done
	)

	# write coverage summary to terminal
	llvm-cov report ${test_objects} ${ignore_paths} \
		--instr-profile="${profdata}" --summary-only

	# generate HTML report
	llvm-cov show ${test_objects} ${ignore_paths} \
		--instr-profile="${profdata}" --use-color \
		--show-instantiations --show-line-counts-or-regions \
		--format=html --output-dir=${outdir}
	echo "HTML report written to: ${outdir}/index.html"
}

clean_raw() {
	# clean raw output so run results don't get mixed
	rm -f default_*.profraw
}

clean() {
	# clean any old output
	clean_raw
	rm -rf "${profdata}" "${outdir}"
}

case "$1" in
	""|run)
		clean_raw
		run
		;;
	clean)
		clean
		;;
	*)
		echo "unknown command!"
		exit 1
		;;
esac
