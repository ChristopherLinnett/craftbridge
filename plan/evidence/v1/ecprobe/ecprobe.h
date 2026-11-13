// C view of ecprobe (Rust staticlib); every returned string is freed with ecprobe_free.
#include <stddef.h>
#include <stdint.h>
char *ecprobe_system_dirs(void);
char *ecprobe_count_faces(const char *dirs_newline_separated);
char *ecprobe_register_dirs(const char *dirs_newline_separated);
char *ecprobe_add_font_data(const uint8_t *ptr, size_t len);
char *ecprobe_fallback_faces(void);
char *ecprobe_face_match(const char *dirs_newline_separated, const char *coretext_faces_tsv, const char *ui_family_names);
void ecprobe_free(char *p);
