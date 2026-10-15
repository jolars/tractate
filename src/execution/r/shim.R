local({
  args <- commandArgs(trailingOnly = TRUE)
  control <- socketConnection("127.0.0.1", port = as.integer(args[[1L]]),
                              blocking = TRUE, open = "a+b")
  on.exit(close(control))
  send <- function(bytes) {
    writeBin(as.integer(length(bytes)), control, size = 4L, endian = "big")
    writeBin(bytes, control)
    flush(control)
  }
  receive <- function() {
    size <- readBin(control, integer(), n = 1L, size = 4L, endian = "big")
    if (length(size) != 1L || size < 1L || size > 16777216L)
      stop("Invalid control frame")
    bytes <- readBin(control, raw(), n = size)
    if (length(bytes) != size) stop("Truncated control frame")
    bytes
  }
  workspace <- new.env(parent = globalenv())
  send(charToRaw("tractate-r/1"))
  repeat {
    request <- receive()
    if (request[[1L]] == as.raw(0L)) break
    if (request[[1L]] != as.raw(1L)) stop("Unknown control command")
    error <- tryCatch({
      expressions <- parse(text = rawToChar(request[-1L]), keep.source = TRUE)
      for (expression in expressions) {
        value <- withVisible(eval(expression, envir = workspace))
        if (value$visible) print(value$value)
      }
      NULL
    }, error = function(error) conditionMessage(error))
    flush(stdout())
    flush(stderr())
    if (is.null(error)) send(as.raw(0L))
    else send(c(as.raw(1L), charToRaw(enc2utf8(error))))
  }
})
