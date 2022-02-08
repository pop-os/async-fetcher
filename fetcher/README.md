# Pop Fetcher

A command-line utility for massively-concurrent and efficient caching of files from the Internet.

## Features

### Fully asynchronous

Designed from the ground up around an asynchronous paradigm

- Concurrently fetches multiple files
- Concurrently fetches multiple parts of a file
- Concurrently fetches from multiple mirrors
- Performs checksum validation in a thread pool

### Configurable

Fetches a wealth of command-line arguments for configuring the behavior of the fetcher. Options defined via the command-line argument are applied to all sources given to the fetcher. Per-source configuration options are also supported when supplying inputs through standard input.

### IPC-driven design

All sources are given to the fetcher through standard input. Each source is defined in [RON syntax](). Frames are denoted by a newline beginning with the `)` character. [An example input can be found here](./sample.ron). Additionally, if the standard output is not a TTY, all events will be written in RON syntax, where frames are delimited by newlines.

### Progress bars

When run interactively (standard output is not a TTY), a progress bar is displayed for each file being actively fetched.

### Checksum validation

Each input source may optionally define a checksum, which will be verified after fetching the file. If the checksum is not a match, the file which was fetched will be deleted. The following algorithms are currently supported:

- MD5
- SHA256

### Only fetch what you need

Checks if previously-fetched files need to be fetched again

- Compare the modified time stamp in the HTTP header with the local file
- Compare the content length in the HTTP header with the file size

Files which are partially-downloaded will also resume from where they left off.
