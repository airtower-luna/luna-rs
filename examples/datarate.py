"""Plot the received data rate over time from one or more LUNA
measurements. The earliest packet receive time is used as 0 for all
series. For example, after doing a measurement with echo enabled, plot
both into one graph for comparison:

```sh
python3 examples/datarate.py -l "Server" -l "Echo" server.tsv echo.tsv
```

With server.tsv containing the server output and echo.tsv the client
output.

"""
import argparse
import csv
import matplotlib.pyplot
from decimal import Decimal
from pathlib import Path


def rate_over_time(datafile: Path, avg_width=0.01):
    """Calculate a rolling average data rate over the
    measurement. `avg_width` sets the width of the time frame for the
    rolling average, relative to the measurement duration.

    """
    with datafile.open(newline='') as tsvfile:
        reader = csv.DictReader(tsvfile, delimiter='\t')
        # Timestamps are converted to int because matplotlib doesn't
        # work with Decimal. The tuples contain receive timestamp and
        # size.
        records: list[tuple[int, int]] = list()
        start = None
        for row in reader:
            rtime = Decimal(row['receive_time'])
            if start is None:
                start = rtime
            size = int(row['size'])
            records.append((int((rtime) * 1_000_000_000), size))
        assert start is not None

    duration = records[-1][0] - records[0][0]
    # Time window width for data rate in ns
    width = duration * avg_width
    half_width = width / 2
    # Select time values for which to calculate data rate with the
    # given width
    x: list[float] = [half_width + records[0][0]]
    while (n := x[-1] + half_width) < (duration + records[0][0]):
        x.append(n)
    y: list[float] = list()
    # Calculate the averages. Because values are ordered we can skip
    # checking records that weren't relevant for previous x (time)
    # values.
    low = 0
    for center in x:
        # set low to the lowest relevant record index
        for i in range(low, len(records)):
            low = i
            if records[i][0] > (center - half_width):
                break
        # sum up size of records within the time window
        b = 0
        for r in records[low:]:
            if r[0] > (center + half_width):
                break
            b += r[1]
        # result: Mbit/s
        y.append(b / width * 1_000 * 8)
    return x, y


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Plot datarate over time from LUNA output')
    parser.add_argument(
        'datafile', nargs='+', type=Path,
        help='TSV file(s) to plot')
    parser.add_argument(
        '--label', '-l', action='append', type=str,
        help='labels, in same order as files')
    parser.add_argument(
        '--show', '-s', action='store_true',
        help='show plot after saving it to file (does nothing if '
        '--outfile is not set)')
    parser.add_argument(
        '--outfile', '-o',
        help='save plot to this file, must have a file suffix recognized '
        'by matplotlib.pyplot (e.g. "pdf")')
    args = parser.parse_args()

    f = matplotlib.pyplot.figure()
    ax = f.subplots()
    plots = [rate_over_time(d) for d in args.datafile]
    # lowest and highest time values over all plots
    x_min = min(p[0][0] for p in plots)
    x_max = max(p[0][-1] for p in plots)
    # plot with time in seconds, with the lowest start time a x=0
    for p in plots:
        ax.plot([(m - x_min) / 1_000_000_000 for m in p[0]], p[1])
    ax.set_xlim(left=0, right=(x_max - x_min) / 1_000_000_000)
    ax.set_ylim(bottom=0)
    ax.set_xlabel('Time [s]')
    ax.set_ylabel('Data rate [MBit/s]')
    labels = args.label
    # If there are unlabeled plots, fill in the file names minus
    # suffix.
    if len(labels) < len(args.datafile):
        labels.extend(d.stem for d in args.datafile[len(labels):])
    ax.legend(labels)

    if args.outfile is not None:
        f.savefig(args.outfile)
    if args.show or args.outfile is None:
        matplotlib.pyplot.show()
    matplotlib.pyplot.close(f)


if __name__ == '__main__':
    main()
