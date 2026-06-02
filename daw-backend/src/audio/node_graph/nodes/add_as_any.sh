#!/bin/bash
for file in *.rs; do
  if [ "$file" = "mod.rs" ]; then
    continue
  fi
  
  echo "Processing $file"
  
  # Create a backup
  cp "$file" "$file.bak"
  
  # Add as_any() method right after as_any_mut()
  awk '
    {
        lines[NR] = $0
        if (/fn as_any_mut\(&mut self\)/) {
            # Found as_any_mut, look for its closing brace
            found_method = NR
        }
        if (found_method > 0 && /^    }$/ && !inserted) {
            closing_brace = NR
            inserted = 1
        }
    }
    END {
        for (i = 1; i <= NR; i++) {
            print lines[i]
            if (i == closing_brace) {
                print ""
                print "    fn as_any(&self) -> &dyn std::any::Any {"
                print "        self"
                print "    }"
            }
        }
    }
  ' "$file.bak" > "$file"
  
  # Verify the change was made
  if grep -q "fn as_any(&self)" "$file"; then
    echo "  ✓ Successfully added as_any() to $file"
    rm "$file.bak"
  else
    echo "  ✗ Failed to add as_any() to $file - restoring backup"
    mv "$file.bak" "$file"
  fi
done
