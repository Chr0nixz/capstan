__capstan_osc7_v1_emit() {
    local __capstan_code=$?
    local LC_ALL=C
    local __capstan_path="${PWD-}" __capstan_uri='' __capstan_char __capstan_hex
    local __capstan_host='__CAPSTAN_AUTHORITY__'
    local __capstan_index
    [[ $__capstan_path == /* && ${#__capstan_path} -le 4096 ]] || return "$__capstan_code"
    for ((__capstan_index=0; __capstan_index<${#__capstan_path}; __capstan_index++)); do
        __capstan_char="${__capstan_path:__capstan_index:1}"
        case "$__capstan_char" in
            [a-zA-Z0-9/._~-]) __capstan_uri+="$__capstan_char" ;;
            *) builtin printf -v __capstan_hex '%%%02X' "'$__capstan_char"
               __capstan_uri+="$__capstan_hex" ;;
        esac
    done
    if (( ${#__capstan_uri} + ${#__capstan_host} + 7 <= 8192 )); then
        builtin printf '\033]7;file://__CAPSTAN_AUTHORITY__%s\033\\' "$__capstan_uri" || :
    fi
    return "$__capstan_code"
}
__capstan_osc7_v1_setup() {
    local __capstan_declaration
    [[ ${__capstan_osc7_v1_installed-} == yes ]] && return 0
    __capstan_declaration=$(declare -p PROMPT_COMMAND 2>/dev/null) || :
    [[ $__capstan_declaration =~ ^declare\ -[^[:space:]]*[Ainr] ]] && return 0
    if [[ $__capstan_declaration =~ ^declare\ -[^[:space:]]*a ]]; then
        PROMPT_COMMAND=(__capstan_osc7_v1_emit "${PROMPT_COMMAND[@]}")
    elif [[ -n ${PROMPT_COMMAND-} ]]; then
        PROMPT_COMMAND=$'__capstan_osc7_v1_emit\n'"$PROMPT_COMMAND"
    else
        PROMPT_COMMAND=__capstan_osc7_v1_emit
    fi
    __capstan_osc7_v1_installed=yes
}
if [[ $- == *i* ]]; then
    __capstan_osc7_v1_setup
fi
unset -f __capstan_osc7_v1_setup
