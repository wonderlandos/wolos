#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# Here-documents: <<EOF interpolates, <<~EOF strips indentation, <<'EOF' is literal.
who = "a b"
cat <<EOF
  plain #{who} and $USER
EOF
cat <<~EOF
  stripped #{who}
    two more spaces
EOF
cat <<'EOF'
  literal #{who} $USER
EOF
tr a-z A-Z <<~EOF | cat
  piped #{who}
EOF
cat <<-EOF > doc.txt
	to a file
	EOF
cat doc.txt
puts "classified after: #{who}"
