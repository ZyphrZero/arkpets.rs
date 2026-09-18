/* Rust owns file I/O and GPU resources; Spine owns skeletons and animation data. */
#include <spine/spine.h>
#include <spine/extension.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

typedef struct {
    spAtlas *atlas;
    spSkeletonData *data;
    spSkeleton *skeleton;
    spAnimationStateData *state_data;
    spAnimationState *state;
    spSkeletonClipping *clip;
    spFloatArray *vertices;
    int completed;
} ArkSpine;

/* Atlas dimensions are supplied by the .atlas file. Images are loaded by Rust. */
void _spAtlasPage_createTexture(spAtlasPage *page, const char *path) { (void)page; (void)path; }
void _spAtlasPage_disposeTexture(spAtlasPage *page) { (void)page; }
char *_spUtil_readFile(const char *path, int *length) { return _spReadFile(path, length); }

static void on_event(spAnimationState *state, spEventType type, spTrackEntry *entry, spEvent *event) {
    ArkSpine *pet = (ArkSpine *)state->userData;
    (void)event;
    if (type == SP_ANIMATION_COMPLETE && entry == spAnimationState_getCurrent(state, 0)) pet->completed = 1;
}

void ark_spine_free(ArkSpine *pet) {
    if (!pet) return;
    if (pet->state) spAnimationState_dispose(pet->state);
    if (pet->state_data) spAnimationStateData_dispose(pet->state_data);
    if (pet->skeleton) spSkeleton_dispose(pet->skeleton);
    if (pet->data) spSkeletonData_dispose(pet->data);
    if (pet->atlas) spAtlas_dispose(pet->atlas);
    if (pet->clip) spSkeletonClipping_dispose(pet->clip);
    if (pet->vertices) spFloatArray_dispose(pet->vertices);
    free(pet);
}

ArkSpine *ark_spine_load(const char *atlas, int atlas_len, const unsigned char *data, int data_len,
                        int json, char *error, int error_len) {
    ArkSpine *pet = (ArkSpine *)calloc(1, sizeof(ArkSpine));
    if (!pet) return NULL;
    pet->atlas = spAtlas_create(atlas, atlas_len, "", NULL);
    if (!pet->atlas) {
        snprintf(error, error_len, "Spine could not parse the atlas");
        ark_spine_free(pet);
        return NULL;
    }
    if (json) {
        spSkeletonJson *reader = spSkeletonJson_create(pet->atlas);
        pet->data = spSkeletonJson_readSkeletonData(reader, (const char *)data);
        if (!pet->data) snprintf(error, error_len, "%s", reader->error ? reader->error : "Invalid skeleton JSON");
        spSkeletonJson_dispose(reader);
    } else {
        spSkeletonBinary *reader = spSkeletonBinary_create(pet->atlas);
        pet->data = spSkeletonBinary_readSkeletonData(reader, data, data_len);
        if (!pet->data) snprintf(error, error_len, "%s", reader->error ? reader->error : "Invalid skeleton binary");
        spSkeletonBinary_dispose(reader);
    }
    if (!pet->data) { ark_spine_free(pet); return NULL; }
    pet->skeleton = spSkeleton_create(pet->data);
    pet->state_data = spAnimationStateData_create(pet->data);
    pet->state_data->defaultMix = 0.3f;
    pet->state = spAnimationState_create(pet->state_data);
    pet->state->userData = pet;
    pet->state->listener = on_event;
    pet->clip = spSkeletonClipping_create();
    pet->vertices = spFloatArray_create(256);
    spSkeleton_updateWorldTransform(pet->skeleton);
    return pet;
}

int ark_spine_animation_count(ArkSpine *pet) { return pet->data->animationsCount; }
const char *ark_spine_animation_name(ArkSpine *pet, int i) { return pet->data->animations[i]->name; }
const char *ark_spine_page(ArkSpine *pet, int index) {
    spAtlasPage *page = pet->atlas->pages;
    while (page && index-- > 0) page = page->next;
    return page ? page->name : NULL;
}
int ark_spine_play(ArkSpine *pet, const char *name, int loop) {
    spAnimation *animation = spSkeletonData_findAnimation(pet->data, name);
    if (!animation) return 0;
    spAnimationState_setAnimation(pet->state, 0, animation, loop);
    pet->completed = 0;
    return 1;
}
int ark_spine_update(ArkSpine *pet, float dt, float mix) {
    pet->completed = 0;
    pet->state_data->defaultMix = mix;
    spAnimationState_update(pet->state, dt);
    spAnimationState_apply(pet->state, pet->skeleton);
    spSkeleton_updateWorldTransform(pet->skeleton);
    return pet->completed;
}

typedef void (*MeshCallback)(void *, int, int, const float *, const float *, int,
                             const unsigned short *, int, float, float, float, float);

void ark_spine_meshes(ArkSpine *pet, void *context, MeshCallback emit) {
    static unsigned short quad[] = {0, 1, 2, 2, 3, 0};
    int i;
    for (i = 0; i < pet->skeleton->slotsCount; ++i) {
        spSlot *slot = pet->skeleton->drawOrder[i];
        spAttachment *attachment = slot->attachment;
        float *vertices = NULL, *uvs = NULL;
        unsigned short *triangles = NULL;
        int count = 0, triangle_count = 0, page_index = 0;
        spAtlasRegion *region = NULL;
        spColor color;
        spAtlasPage *page;
        if (!attachment || !slot->bone->active || slot->color.a == 0) {
            spSkeletonClipping_clipEnd(pet->clip, slot);
            continue;
        }
        if (attachment->type == SP_ATTACHMENT_CLIPPING) {
            spSkeletonClipping_clipStart(pet->clip, slot, (spClippingAttachment *)attachment);
            continue;
        }
        if (attachment->type == SP_ATTACHMENT_REGION) {
            spRegionAttachment *a = (spRegionAttachment *)attachment;
            spFloatArray_setSize(pet->vertices, 8);
            vertices = pet->vertices->items;
            spRegionAttachment_computeWorldVertices(a, slot->bone, vertices, 0, 2);
            count = 8; uvs = a->uvs; triangles = quad; triangle_count = 6;
            color = a->color; region = (spAtlasRegion *)a->rendererObject;
        } else if (attachment->type == SP_ATTACHMENT_MESH) {
            spMeshAttachment *a = (spMeshAttachment *)attachment;
            count = a->super.worldVerticesLength;
            spFloatArray_setSize(pet->vertices, count);
            vertices = pet->vertices->items;
            spVertexAttachment_computeWorldVertices(&a->super, slot, 0, count, vertices, 0, 2);
            uvs = a->uvs; triangles = a->triangles; triangle_count = a->trianglesCount;
            color = a->color; region = (spAtlasRegion *)a->rendererObject;
        } else {
            spSkeletonClipping_clipEnd(pet->clip, slot);
            continue;
        }
        if (spSkeletonClipping_isClipping(pet->clip)) {
            spSkeletonClipping_clipTriangles(pet->clip, vertices, count, triangles, triangle_count, uvs, 2);
            vertices = pet->clip->clippedVertices->items; count = pet->clip->clippedVertices->size;
            uvs = pet->clip->clippedUVs->items;
            triangles = pet->clip->clippedTriangles->items; triangle_count = pet->clip->clippedTriangles->size;
        }
        page = pet->atlas->pages;
        while (page && page != region->page) { page = page->next; ++page_index; }
        if (count && triangle_count) emit(context, page_index, slot->data->blendMode,
            vertices, uvs, count / 2, triangles, triangle_count,
            color.r * slot->color.r, color.g * slot->color.g, color.b * slot->color.b, color.a * slot->color.a);
        spSkeletonClipping_clipEnd(pet->clip, slot);
    }
    spSkeletonClipping_clipEnd2(pet->clip);
}
