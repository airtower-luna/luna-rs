"""Example evaluation script which calculates the inter arrival times
(IAT, time between the arrival of two consequtive packets during the
measurement) of the packets listed in the input file and creates a
histogram showing how frequently different IATs occurred. With a fixed
send interval this provides an indication of how stable transfer times
are, if packet aggregation occurs, and similar.

The input file must contain tab-separated LUNA output, example shell
commands to create such a file:

luna-rs server >test.tsv &
server_pid="$!"
luna-rs client -e -O count=200 -O usec=30
kill $server_pid
wait

Then view the histogram for the resulting test.tsv file:

python3 examples/iat-histogram.py test.tsv

"""
import argparse
import csv
import matplotlib.pyplot
from decimal import Decimal
from pathlib import Path


def plot_from_file(datafile: Path):
    with datafile.open(newline='') as tsvfile:
        reader = csv.DictReader(tsvfile, delimiter='\t')
        prev = None
        # Inter arrival times in ns, matplotlib.pyplot can't work with
        # Decimal. Converting to float would also work, if the loss in
        # precision is acceptable (depends on context).
        iats: list[int] = list()
        for row in reader:
            current = Decimal(row['receive_time'])
            if prev is not None:
                iats.append(int((current - prev) * 1_000_000_000))
            prev = current

    f = matplotlib.pyplot.figure()
    ax = f.subplots()
    ax.hist(iats, bins=50, align='mid', density=True)
    ax.set_xlabel('Inter arrival time [ns]')
    ax.set_ylabel('Occurrence')
    return f


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Plot inter arrival times from LUNA output')
    parser.add_argument(
        'datafile', nargs='+', type=Path,
        help='TSV file(s) to plot')
    parser.add_argument(
        '--show', '-s', action='store_true',
        help='show plot after saving it to file (does nothing if '
        '--outform is not set)')
    parser.add_argument(
        '--outform', '-o',
        help='save plot in this format, must be a file suffix recognized '
        'by matplotlib.pyplot (e.g. "pdf")')
    args = parser.parse_args()

    for d in args.datafile:
        f = plot_from_file(d)
        if args.outform is not None:
            f.savefig(d.with_suffix(f'.{args.outform}'))
        if args.show or args.outform is None:
            matplotlib.pyplot.show()
        matplotlib.pyplot.close(f)


if __name__ == '__main__':
    main()
