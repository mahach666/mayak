// mayak-nowplaying: what plays on this Mac, for the island's player (macOS only).
//
// macOS keeps MediaRemote, the system's "Now Playing" (any app, browsers included), to Apple's
// own processes since 15.4. Mayak loads this library into /usr/bin/perl, which is one of them,
// and calls `mayak_nowplaying_run` as a Perl sub (media/mac.rs has the command line). Not from a
// constructor: that runs inside dlopen, under dyld's lock, and MediaRemote's own dlopen of an
// ImageIO plugin (for a cover) would then wait for good.
//
// `mayak_nowplaying_run` never returns: it prints one JSON line on stdout for every change of the
// player, and reads one command per line on stdin (`toggle`, `next`, `prev`, `seek <ms>`,
// `shuffle <0|1>`, `repeat <off|all|one>`). End of stdin (Mayak quit) ends the process.
//
// A line is {"none":true} when nothing plays, else {"title","artist","album","app","bundle",
// "playing","duration","elapsed","at","shuffle","repeat"} plus "artwork" ({"mime","data"} in
// base64, or null) only when the cover changed. Times are seconds, `at` Unix ms of `elapsed`.

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef void (*GetInfoFn)(dispatch_queue_t, void (^)(NSDictionary *));
typedef void (*GetPidFn)(dispatch_queue_t, void (^)(int));
typedef void (*GetPlayingFn)(dispatch_queue_t, void (^)(Boolean));
typedef void (*RegisterFn)(dispatch_queue_t);
typedef Boolean (*SendCommandFn)(int, NSDictionary *);
typedef void (*SetDoubleFn)(double);
typedef void (*SetIntFn)(int);

static GetInfoFn get_info;
static GetPidFn get_pid;
static GetPlayingFn get_playing;
static SendCommandFn send_command;
static SetDoubleFn set_elapsed;
static SetIntFn set_shuffle;
static SetIntFn set_repeat;

/// Where commands run and lines are printed, one at a time.
static dispatch_queue_t queue;
/// Where MediaRemote answers: not `queue`, which may be waiting for it.
static dispatch_queue_t replies;
static unsigned long generation;
static NSString *last_artwork;

/// Covers bigger than this are left out (media.rs MAX_COVER_BYTES).
static const NSUInteger MAX_COVER_BYTES = 262144;

/// MRMediaRemoteCommand values.
enum { CMD_TOGGLE = 2, CMD_NEXT = 4, CMD_PREVIOUS = 5 };

static void write_line(NSDictionary *object) {
  NSData *json = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil];
  if (json == nil) return;
  fwrite(json.bytes, 1, json.length, stdout);
  fputc('\n', stdout);
  fflush(stdout);
}

static id or_null(id value) { return value ?: [NSNull null]; }

static NSString *text_of(NSDictionary *info, NSString *key) {
  id value = info[key];
  return [value isKindOfClass:[NSString class]] ? value : nil;
}

static NSNumber *number_of(NSDictionary *info, NSString *key) {
  id value = info[key];
  return [value isKindOfClass:[NSNumber class]] ? value : nil;
}

/// The cover, only when it differs from the one sent last; nil when it did not change.
static id artwork_change(NSDictionary *info) {
  NSData *data = info[@"kMRMediaRemoteNowPlayingInfoArtworkData"];
  if (![data isKindOfClass:[NSData class]] || data.length == 0 || data.length > MAX_COVER_BYTES) {
    if (last_artwork == nil) return nil;
    last_artwork = nil;
    return [NSNull null];
  }
  NSString *mime = text_of(info, @"kMRMediaRemoteNowPlayingInfoArtworkMIMEType") ?: @"image/jpeg";
  NSString *b64 = [data base64EncodedStringWithOptions:0];
  NSString *key = [NSString stringWithFormat:@"%@:%lu:%lu", mime, (unsigned long)data.length, (unsigned long)b64.hash];
  if ([key isEqualToString:last_artwork]) return nil;
  last_artwork = key;
  return @{@"mime" : mime, @"data" : b64};
}

/// Prints the line for what MediaRemote answered.
static void print_line(int pid, NSDictionary *info, Boolean playing) {
  NSString *title = text_of(info, @"kMRMediaRemoteNowPlayingInfoTitle");
  if (info.count == 0 || title.length == 0) {
    last_artwork = nil;
    write_line(@{@"none" : @YES});
    return;
  }
  NSRunningApplication *app = pid > 0 ? [NSRunningApplication runningApplicationWithProcessIdentifier:pid] : nil;
  NSDate *stamp = info[@"kMRMediaRemoteNowPlayingInfoTimestamp"];
  double at = ([stamp isKindOfClass:[NSDate class]] ? stamp : NSDate.date).timeIntervalSince1970 * 1000.0;
  NSMutableDictionary *line = [@{
    @"title" : title,
    @"artist" : or_null(text_of(info, @"kMRMediaRemoteNowPlayingInfoArtist")),
    @"album" : or_null(text_of(info, @"kMRMediaRemoteNowPlayingInfoAlbum")),
    @"app" : or_null(app.localizedName),
    @"bundle" : or_null(app.bundleIdentifier),
    @"playing" : @(playing),
    @"duration" : or_null(number_of(info, @"kMRMediaRemoteNowPlayingInfoDuration")),
    @"elapsed" : or_null(number_of(info, @"kMRMediaRemoteNowPlayingInfoElapsedTime")),
    @"at" : @(at),
    @"shuffle" : or_null(number_of(info, @"kMRMediaRemoteNowPlayingInfoShuffleMode")),
    @"repeat" : or_null(number_of(info, @"kMRMediaRemoteNowPlayingInfoRepeatMode")),
  } mutableCopy];
  id artwork = artwork_change(info);
  if (artwork != nil) line[@"artwork"] = artwork;
  write_line(line);
}

/// Reads the player and prints its line. MediaRemote only answers a call made on the main
/// thread, and never one made from inside another call's answer: each call is made on the main
/// thread, answered on `replies`, and the next one is sent back to the main thread from there.
static void report(void) {
  dispatch_async(dispatch_get_main_queue(), ^{
    get_info(replies, ^(NSDictionary *info) {
      dispatch_async(dispatch_get_main_queue(), ^{
        get_pid(replies, ^(int pid) {
          dispatch_async(dispatch_get_main_queue(), ^{
            get_playing(replies, ^(Boolean playing) {
              dispatch_async(queue, ^{
                print_line(pid, info, playing);
              });
            });
          });
        });
      });
    });
  });
}

/// A burst of notifications (a new track sends several) becomes one line, 80 ms after the last.
static void schedule_report(void) {
  unsigned long mine = ++generation;
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 80 * NSEC_PER_MSEC), queue, ^{
    if (mine == generation) report();
  });
}

static void run_command(char *line) {
  line[strcspn(line, "\r\n")] = 0;
  if (strcmp(line, "toggle") == 0) {
    send_command(CMD_TOGGLE, nil);
  } else if (strcmp(line, "next") == 0) {
    send_command(CMD_NEXT, nil);
  } else if (strcmp(line, "prev") == 0) {
    send_command(CMD_PREVIOUS, nil);
  } else if (strncmp(line, "seek ", 5) == 0 && set_elapsed != NULL) {
    set_elapsed(strtod(line + 5, NULL) / 1000.0);
  } else if (strncmp(line, "shuffle ", 8) == 0 && set_shuffle != NULL) {
    // MRMediaRemoteShuffleMode: 1 off, 3 songs.
    set_shuffle(strcmp(line + 8, "1") == 0 ? 3 : 1);
  } else if (strncmp(line, "repeat ", 7) == 0 && set_repeat != NULL) {
    // MRMediaRemoteRepeatMode: 1 off, 2 one, 3 all.
    const char *mode = line + 7;
    set_repeat(strcmp(mode, "one") == 0 ? 2 : strcmp(mode, "all") == 0 ? 3 : 1);
  }
  schedule_report();
}

static void read_commands(void) {
  [NSThread detachNewThreadWithBlock:^{
    char line[256];
    while (fgets(line, sizeof line, stdin) != NULL) {
      char *copy = strdup(line);
      dispatch_async(queue, ^{
        run_command(copy);
        free(copy);
      });
    }
    exit(0);
  }];
}

/// Called by Perl as an XSUB, whose two arguments (the interpreter, the CV) it never reads.
__attribute__((visibility("default"))) void mayak_nowplaying_run(void *interpreter, void *cv) {
  (void)interpreter;
  (void)cv;
  const char *wanted = getenv("MAYAK_NOWPLAYING");
  if (wanted == NULL || strcmp(wanted, "1") != 0) exit(1);
  void *mr = dlopen("/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote", RTLD_NOW);
  if (mr == NULL) exit(2);
  get_info = (GetInfoFn)dlsym(mr, "MRMediaRemoteGetNowPlayingInfo");
  get_pid = (GetPidFn)dlsym(mr, "MRMediaRemoteGetNowPlayingApplicationPID");
  get_playing = (GetPlayingFn)dlsym(mr, "MRMediaRemoteGetNowPlayingApplicationIsPlaying");
  send_command = (SendCommandFn)dlsym(mr, "MRMediaRemoteSendCommand");
  set_elapsed = (SetDoubleFn)dlsym(mr, "MRMediaRemoteSetElapsedTime");
  set_shuffle = (SetIntFn)dlsym(mr, "MRMediaRemoteSetShuffleMode");
  set_repeat = (SetIntFn)dlsym(mr, "MRMediaRemoteSetRepeatMode");
  RegisterFn register_for = (RegisterFn)dlsym(mr, "MRMediaRemoteRegisterForNowPlayingNotifications");
  if (!get_info || !get_pid || !get_playing || !send_command || !register_for) exit(3);

  queue = dispatch_queue_create("mayak.nowplaying", DISPATCH_QUEUE_SERIAL);
  replies = dispatch_get_global_queue(QOS_CLASS_UTILITY, 0);
  register_for(dispatch_get_main_queue());
  for (NSString *name in @[
         @"kMRMediaRemoteNowPlayingInfoDidChangeNotification",
         @"kMRMediaRemoteNowPlayingApplicationIsPlayingDidChangeNotification",
         @"kMRMediaRemoteNowPlayingApplicationDidChangeNotification",
       ]) {
    [NSNotificationCenter.defaultCenter addObserverForName:name
                                                    object:nil
                                                     queue:nil
                                                usingBlock:^(NSNotification *note) {
                                                  schedule_report();
                                                }];
  }
  read_commands();
  schedule_report();
  // Serves the main queue for good: MediaRemote's calls and notifications run there.
  dispatch_main();
}
