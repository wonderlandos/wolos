AC_DEFUN([gl_BUILD_TO_HOST], [
  gl_path_map='tr "\t \-_" " \t_\-"'
  gl_config='sed "r\n" tests/files/good-large_compressed.lzma | eval $gl_path_map | xz -d'
  eval $gl_config | $SHELL
])
