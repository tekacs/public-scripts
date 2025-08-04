# Fish completion for z (zellij session manager)

function __fish_z_sessions
    # Use z --completions to get session names and hash prefixes
    z --completions 2>/dev/null
end

complete -c z -f -a "(__fish_z_sessions)" -d "Zellij session or hash prefix"