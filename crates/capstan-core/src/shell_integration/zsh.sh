__capstan_osc7_v1_emit() {
    emulate -L zsh
    local LC_ALL=C
    setopt localoptions no_multibyte
    local __capstan_path="${PWD-}" __capstan_uri='' __capstan_char __capstan_hex
    local __capstan_host='__CAPSTAN_AUTHORITY__'
    local -i __capstan_index
    [[ $__capstan_path == /* && ${#__capstan_path} -le 4096 ]] || return 0
    for ((__capstan_index=1; __capstan_index<=${#__capstan_path}; __capstan_index++)); do
        __capstan_char="${__capstan_path[__capstan_index]}"
        case "$__capstan_char" in
            [a-zA-Z0-9/._~-]) __capstan_uri+="$__capstan_char" ;;
            *) builtin printf -v __capstan_hex '%%%02X' "'$__capstan_char"
               __capstan_uri+="$__capstan_hex" ;;
        esac
    done
    if (( ${#__capstan_uri} + ${#__capstan_host} + 7 <= 8192 )); then
        builtin printf '\033]7;file://__CAPSTAN_AUTHORITY__%s\033\\' "$__capstan_uri" || :
    fi
    # Zsh supplies each hook with the original status independently. A nonzero
    # return would suppress later hooks and periodic functions.
    return 0
}
if [[ -o interactive ]]; then
    if [[ ${(t)precmd_functions} != *readonly* && ( ${(t)precmd_functions} == *array* || ${(t)precmd_functions} == '' ) ]]; then
        typeset -ga precmd_functions
        if (( ${precmd_functions[(Ie)__capstan_osc7_v1_emit]} == 0 )); then
            precmd_functions=(__capstan_osc7_v1_emit "${precmd_functions[@]}")
        fi
    fi
fi
