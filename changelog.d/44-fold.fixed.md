- A release build no longer folds a constant its analysis had not finished
  proving. Constant propagation bails out at a bound, and until it settles its
  states are optimistic, so a `Branch` could be rewritten into a `Goto` the
  program never asked for; a run that reaches the bound now folds nothing and
  reports `SL0700` instead. The bound has never been reached by a real
  program, and reaching it would mean the bound is wrong.
