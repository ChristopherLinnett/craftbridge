#!/usr/bin/env python3
import re, sys
DOCS = "/tmp/craftbridge-v7/docs/"
def norm(s):
    # normalize: curly quotes -> straight, collapse whitespace, fix common HTML-quotes
    s = s.replace('\u2019', "'").replace('\u2018', "'").replace('\u201c', '"').replace('\u201d', '"')
    s = re.sub(r'\s+', ' ', s)
    return s.strip()
files = ["play-console-fgs.txt","awake.txt","service-types.txt","fgs-timeout.txt","service-ref.txt","serviceinfo-ref.txt","v14-required.txt","v15-changes.txt","bgstart.txt","fgs-guide.txt","docs_tmp.html"]
data = {f: norm(open(DOCS+f, encoding='utf-8', errors='replace').read()) for f in files}
quotes = [
 ("When your apps target Android 14 and above, you'll need to declare any foreground service types that you use in a new declaration on the App content page", "play"),
 ("For each foreground service type you declare, you'll need to do the following", "play"),
 ("Provide a description of the app functionality that is using each foreground service type", "play"),
 ("Describe the user impact if: the task is deferred by the system (does not start immediately); and/or the task is interrupted by the system (paused and/or restarted)", "play"),
 ("Include a link to a video demonstrating each foreground service feature", "play"),
 ("Choose your specific use case for each foreground service type", "play"),
 ("Local processing : Import or export", "play"),
 ("Includes migration of files from SD card", "play"),
 ("Show the user manually triggering an import or export of files locally", "play"),
 ("Local processing : Other", "play"),
 ("Use for specifically user-initiated work and not for regular system or server-initiated tasks. For example, encryption, resizing.", "play"),
 ("Media transcoding", "play"),
 ("Performing time-consuming operations on media assets, like converting media to different formats", "play"),
 ("we recommend using the user-initiated data transfer job instead of the dataSync foreground service type", "play"),
 ("Network transfer : Backup and restore", "play"),
 ("Use for specifically user-initiated features", "play"),
 ("Would it be detrimental to the user experience if the device suspends while the foreground service is running and the device screen is off?", "awake"),
 ("If Yes, you might need to use a wake lock. However, you should still check to see if you're already using an API or doing an action that declares a wake lock on your behalf, as discussed in Actions that keep the device awake.", "awake"),
 ("If your app is doing any of the following, you don't need to set a wake lock yourself", "awake"),
 ("If you're using task scheduling APIs or libraries such as WorkManager, JobScheduler, or DownloadManager, the system or library acquires a wake lock that is attributed to your app", "awake"),
 ("If you're using Media3 ExoPlayer, you can use ExoPlayer.setWakeMode() to have the player set a wake lock for you", "awake"),
 ("Can only run for a short period of time (about 3 minutes)", "types"),
 ("The Service.onTimeout() callback doesn't exist on Android 13 and lower. If the same service runs on such devices, it doesn't receive a timeout, nor does it ANR.", "types"),
 ("Import or export operations", "types"),
 ("Local file processing", "types"),
 ("These values and corresponding use cases are reviewed when you submit your app in the Google Play Console.", "types"),
 ("Service for performing time-consuming operations on media assets, like converting media to different formats. The system allows this service a limited time to run; under normal circumstances, this time limit would be 6 hours out of every 24", "types"),
 ("Policy > App content", "types"),
 ("doesn't finish even after it's timed out, the app will be declared an ANR after a short grace period of several seconds", "serv"),
 ("will also be called when a foreground service of type FOREGROUND_SERVICE_TYPE_SHORT_SERVICE times out", "serv"),
 ("that time counter begins as soon as the foreground service starts", "serv"),
 ("if it fails to do so, the app will crash", "serv"),
 ("do not need to implement both of the callbacks", "serv"),
 ("Added in API level 34", "serv"),
 ("Added in API level 35", "serv"),
 ("Added in API level 35", "info"),
 ("can be also used on prior android versions (just like other new foreground service types can be used)", "info"),
 ("has time limit of 6 hours starting from Android version VANILLA_ICE_CREAM", "info"),
 ("it will never be called", "info"),
 ("has the full six hours after the app goes to the background", "timeout"),
 ("a total of 6 hours in a 24-hour period", "timeout"),
 ("The mediaProcessing foreground service type was added in Android 15.", "timeout"),
 ("you cannot start another dataSync foreground service", "timeout"),
 ("unless the user has brought your app to the foreground (which resets the timer)", "timeout"),
 ("If an app targets Android 15 or higher, the system places restrictions on how long certain foreground services are allowed to run while your app is in the background", "timeout"),
 ("in addition to requesting the FOREGROUND_SERVICE permission", "v14"),
 ("is available if your app targets Android 15 or higher", "v15"),
 ("Encoding or decoding a media file", "v15"),
 ("Apps that target Android 12 (API level 31) or higher can't start foreground services while the app is running in the background", "bgstart"),
]
fmap = {"play":"play-console-fgs.txt","awake":"awake.txt","types":"service-types.txt","serv":"service-ref.txt","info":"serviceinfo-ref.txt","timeout":"fgs-timeout.txt","v14":"v14-required.txt","v15":"v15-changes.txt","bgstart":"bgstart.txt"}
bad = 0
for q, key in quotes:
    fn = fmap[key]
    found = norm(q) in data[fn]
    others = [f for f,d in data.items() if norm(q) in d]
    if not found:
        bad += 1
        print(f"NOT FOUND in {fn}: {q[:80]}...  (anywhere: {others})")
print(f"checked {len(quotes)} quotes, {bad} not found in the expected file")
