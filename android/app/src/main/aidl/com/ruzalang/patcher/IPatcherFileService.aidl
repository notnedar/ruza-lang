package com.ruzalang.patcher;

interface IPatcherFileService {
    void destroy() = 16777114;
    boolean fileExists(String path) = 1;
    long fileSize(String path) = 2;
    String sha256File(String path) = 3;
    String listDir(String path) = 4;
    String copyFileAtomic(String srcPath, String dstPath) = 5;
    String findRunningPids(String packageName) = 6;
    String getPackageVersion(String packageName) = 7;
    boolean isPackageInstalled(String packageName) = 8;
    String findActiveVStringBundle(String rawBundleDir, in byte[] key4096) = 9;
    String findActiveStaticUiBundle(String defaultBundleDir, String preferredName) = 10;
}
