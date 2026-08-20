- A debug or continuous-integration build no longer aborts on a program that is
  still worth optimizing after the last pipeline round. That was an assertion
  about a legitimate outcome; the pipeline stops and keeps what it achieved.
