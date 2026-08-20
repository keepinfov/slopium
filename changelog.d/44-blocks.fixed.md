- A terminator naming a block that does not exist is reported as `SL0700`
  rather than crashing the compiler in a release build, where the verification
  that would have caught it does not run.
